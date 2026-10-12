//! Shared admission: when an HTTP request may start.
//!
//! An [`Admission`] instance is one **budget scope**: every client that
//! shares it, including clones and clients derived with
//! `HTTPClient::with_credentials`, draws from the same windows. It enforces
//! a versioned [`QuotaProfile`] locally. It cannot see requests made by other
//! processes, other applications or other SDK instances using the same API
//! key or account.
//!
//! Admission decides only *when* a request may start. It performs no retry
//! and knows nothing about deadlines beyond the wait bound it is given.
//!
//! # The default profile
//!
//! [`QuotaProfile::kite_v3`] encodes the limits documented in
//! `kite:exceptions.md:45-58`, as configuration rather than verified broker
//! truth:
//!
//! | Class | Endpoints | Windows |
//! |---|---|---|
//! | `Quote` | `/quote`, `/quote/ohlc`, `/quote/ltp` | 1 per second |
//! | `Historical` | `/instruments/historical/{instrument_token}/{interval}` | 3 per second |
//! | `OrderPlacement` | `POST /orders/{variety}` | 10 per second, 400 per minute, 5 000 per IST day |
//! | `OrderModification` | `PUT /orders/{variety}/{order_id}` | 10 per second, 25 modifications per order per IST day |
//! | `Standard` | every other documented endpoint | 10 per second |
//!
//! The IST day runs from 00:00 to 24:00 UTC+05:30. An endpoint without a
//! known class is admitted at the profile's smallest rate, never an
//! unlimited one. Admission waits are bounded in count ([`AdmissionLimits`],
//! `B-HTTP-07`) and time (`B-HTTP-06`), cancellable by dropping the waiting
//! future, and a grant that is dropped before its request is dispatched
//! returns its capacity.
//!
//! # Priority lanes
//!
//! Every request is admitted on one of two lanes, [`Priority::Standard`] or
//! [`Priority::Control`]. Within a class, the `Standard` lane may fill each
//! window only up to its limit minus the class's **control reserve**
//! (`B-HTTP-15`, a field of [`AdmissionLimits`]: 2 per second on the
//! `Standard` class by default, 0 elsewhere), while the `Control` lane may
//! fill the whole window. The reserve keeps the last slots of each second
//! for traffic the caller marks as control, such as cancellations and the
//! reads that confirm them; the SDK never decides which traffic that is. A
//! reserve of 0 makes the lanes equal.
//!
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::time::Instant;

use crate::kite::obs::schema::{Endpoint, Method, PriorityLabel};
use crate::kite::protocol::OrderId;

/// Version of the default quota profile: the documentation page it encodes,
/// the date that page was accessed, and a revision that increases whenever
/// the encoding of that same page changes. A version without a `+r` suffix is
/// revision 1, and the revision restarts at 1 when the page is accessed again
/// on a new date. Revision 2 adds the historical candle class.
pub const QUOTA_PROFILE_VERSION: &str = "kite-connect-v3/exceptions.md@2026-09-23+r2";

/// IST offset in seconds, for the daily ceiling's day boundary.
const IST_OFFSET_SECONDS: i64 = 5 * 3600 + 30 * 60;

/// A rate class of the quota profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RateClass {
    /// Market quote endpoints.
    Quote,
    /// Historical candle data.
    Historical,
    /// Order placement.
    OrderPlacement,
    /// Order modification.
    OrderModification,
    /// Every other documented endpoint.
    Standard,
    /// An endpoint without a known class; admitted at the smallest rate.
    Unclassified,
}

impl RateClass {
    /// Number of classes, the length of the per-class reserve table.
    const COUNT: usize = 6;

    /// The class of an HTTP operation.
    pub fn of(method: Method, endpoint: Endpoint) -> Self {
        match (method, endpoint) {
            (_, Endpoint::Quote | Endpoint::QuoteOhlc | Endpoint::QuoteLtp) => Self::Quote,
            (_, Endpoint::InstrumentsHistorical) => Self::Historical,
            (Method::Post, Endpoint::OrdersVariety) => Self::OrderPlacement,
            (Method::Put, Endpoint::OrdersVarietyId) => Self::OrderModification,
            (_, Endpoint::Unknown) => Self::Unclassified,
            _ => Self::Standard,
        }
    }

    /// Position in the per-class reserve table; the match keeps the table
    /// and the enum in step.
    const fn index(self) -> usize {
        match self {
            Self::Quote => 0,
            Self::Historical => 1,
            Self::OrderPlacement => 2,
            Self::OrderModification => 3,
            Self::Standard => 4,
            Self::Unclassified => 5,
        }
    }
}

/// The admission lane of a request.
///
/// Every HTTP operation is admitted on a lane. `Standard` traffic may use a
/// class's rate minus its control reserve (`B-HTTP-15`); `Control` traffic
/// may use the whole rate. Order cancellation and GTT deletion are `Control`
/// unless the caller marks them otherwise; every other operation is
/// `Standard` unless the caller marks it `Control` with
/// `HTTPClient::with_priority`. The SDK never decides which traffic is
/// control.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Priority {
    /// Ordinary traffic: may not take a class's reserved slots.
    Standard,
    /// Control traffic: may use a class's whole rate.
    Control,
}

impl Priority {
    /// The lane an operation takes when the caller sets none: `Control` for
    /// `DELETE /orders/{variety}/{order_id}` and `DELETE /gtt/triggers/{id}`,
    /// `Standard` for everything else.
    pub(crate) fn default_for(method: Method, endpoint: Endpoint) -> Self {
        match (method, endpoint) {
            (Method::Delete, Endpoint::OrdersVarietyId | Endpoint::GttTriggersId) => Self::Control,
            _ => Self::Standard,
        }
    }
}

impl From<Priority> for PriorityLabel {
    fn from(priority: Priority) -> Self {
        match priority {
            Priority::Standard => Self::Standard,
            Priority::Control => Self::Control,
        }
    }
}

/// `limit` admissions per `period`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    /// Admissions permitted per period.
    pub limit: u32,
    /// Window length.
    pub period: Duration,
}

impl Window {
    /// A window of `limit` per `period`.
    pub const fn new(limit: u32, period: Duration) -> Self {
        Self { limit, period }
    }

    fn per_second(self) -> f64 {
        self.limit as f64 / self.period.as_secs_f64()
    }
}

/// Why a quota profile or admission configuration was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum QuotaError {
    /// A window has a zero limit or a period under 1 ms (`docs/contract.md §3.6`).
    InvalidWindow,
    /// A class has no window.
    MissingWindows(RateClass),
    /// The order-placement class exceeds 10 per second (`docs/contract.md §3.6`).
    PlacementAboveTenPerSecond,
    /// The daily ceiling or modification limit is zero.
    InvalidCeiling,
    /// Windows were given for `RateClass::Unclassified`, which takes the
    /// profile's smallest rate instead.
    UnclassifiedWindows,
    /// A bound is outside its permitted range.
    Bound(&'static str),
}

impl fmt::Display for QuotaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid quota configuration: {self:?}")
    }
}

impl std::error::Error for QuotaError {}

/// A validated, versioned quota profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuotaProfile {
    version: String,
    quote: Vec<Window>,
    historical: Vec<Window>,
    placement: Vec<Window>,
    modification: Vec<Window>,
    standard: Vec<Window>,
    daily_order_ceiling: u32,
    modifications_per_order: u32,
}

impl QuotaProfile {
    /// The documented Kite Connect v3 limits (see the module table).
    pub fn kite_v3() -> Self {
        let s = Duration::from_secs(1);
        Self {
            version: QUOTA_PROFILE_VERSION.to_string(),
            quote: vec![Window::new(1, s)],
            historical: vec![Window::new(3, s)],
            placement: vec![
                Window::new(10, s),
                Window::new(400, Duration::from_secs(60)),
            ],
            modification: vec![Window::new(10, s)],
            standard: vec![Window::new(10, s)],
            daily_order_ceiling: 5000,
            modifications_per_order: 25,
        }
    }

    /// The profile with a different version label. Give a changed profile
    /// its own version, so that what is in force can be told apart from
    /// [`QUOTA_PROFILE_VERSION`].
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// The profile with `windows` for `class`, validated: at least one
    /// window, each with a positive limit and a period of at least 1 ms, and
    /// no more than 10 per second for order placement. `Unclassified` has
    /// no windows of its own: it takes the profile's smallest rate.
    ///
    /// ```
    /// use std::time::Duration;
    /// use manja::kite::connect::admission::{QuotaProfile, RateClass, Window};
    ///
    /// let profile = QuotaProfile::kite_v3()
    ///     .with_version("desk-quota-1")
    ///     .with_windows(RateClass::Standard, vec![Window::new(5, Duration::from_secs(1))])
    ///     .unwrap();
    /// assert_eq!(profile.windows(RateClass::Standard)[0].limit, 5);
    /// ```
    pub fn with_windows(
        mut self,
        class: RateClass,
        windows: Vec<Window>,
    ) -> Result<Self, QuotaError> {
        if windows.is_empty() {
            return Err(QuotaError::MissingWindows(class));
        }
        if windows
            .iter()
            .any(|w| w.limit == 0 || w.period < Duration::from_millis(1))
        {
            return Err(QuotaError::InvalidWindow);
        }
        let slot = match class {
            RateClass::Quote => &mut self.quote,
            RateClass::Historical => &mut self.historical,
            RateClass::OrderPlacement => {
                if windows.iter().any(|w| w.per_second() > 10.0) {
                    return Err(QuotaError::PlacementAboveTenPerSecond);
                }
                &mut self.placement
            }
            RateClass::OrderModification => &mut self.modification,
            RateClass::Standard => &mut self.standard,
            RateClass::Unclassified => return Err(QuotaError::UnclassifiedWindows),
        };
        *slot = windows;
        Ok(self)
    }

    /// The profile with a different daily ceiling of order placements;
    /// zero is refused.
    pub fn with_daily_order_ceiling(mut self, orders: u32) -> Result<Self, QuotaError> {
        if orders == 0 {
            return Err(QuotaError::InvalidCeiling);
        }
        self.daily_order_ceiling = orders;
        Ok(self)
    }

    /// The profile with a different limit of modifications per order per
    /// IST day; zero is refused.
    pub fn with_modifications_per_order(mut self, limit: u32) -> Result<Self, QuotaError> {
        if limit == 0 {
            return Err(QuotaError::InvalidCeiling);
        }
        self.modifications_per_order = limit;
        Ok(self)
    }

    /// Profile version.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Windows of `class`. An unclassified endpoint gets the single window
    /// with the smallest rate in the profile.
    pub fn windows(&self, class: RateClass) -> Vec<Window> {
        match class {
            RateClass::Quote => self.quote.clone(),
            RateClass::Historical => self.historical.clone(),
            RateClass::OrderPlacement => self.placement.clone(),
            RateClass::OrderModification => self.modification.clone(),
            RateClass::Standard => self.standard.clone(),
            RateClass::Unclassified => {
                let all = [
                    &self.quote,
                    &self.historical,
                    &self.placement,
                    &self.modification,
                    &self.standard,
                ];
                let slowest = all
                    .iter()
                    .flat_map(|w| w.iter())
                    .min_by(|a, b| a.per_second().total_cmp(&b.per_second()))
                    .copied()
                    .expect("validated profiles have windows");
                vec![slowest]
            }
        }
    }

    /// Placements permitted per IST day.
    pub fn daily_order_ceiling(&self) -> u32 {
        self.daily_order_ceiling
    }

    /// Modifications permitted per order per IST day.
    pub fn modifications_per_order(&self) -> u32 {
        self.modifications_per_order
    }
}

/// Validated admission bounds.
///
/// | Bound | Default | Range |
/// |---|---|---|
/// | `B-HTTP-06` admission wait | 5 s | 1 ms ..= 60 s |
/// | `B-HTTP-07` waiters per scope | 256 | 1 ..= 4 096 |
/// | `B-HTTP-15` control reserve per class, per second | 2 (`Standard`), 0 (every other class) | 0 ..= 5 |
///
/// The control reserve is the part of a class's rate that only
/// [`Priority::Control`] traffic may use. For a window of period `p` it
/// reserves `reserve × p` slots, rounded up, so a reserve of 2 per second
/// leaves `Standard` traffic 8 of 10 per second and 280 of 400 per minute.
/// Ceilings (the daily order ceiling and the per-order modification limit)
/// are counts, not rates, and have no reserve. A reserve at or above a
/// window's rate leaves the class no `Standard` capacity: every `Standard`
/// request in it is refused at once with
/// [`AdmissionError::NoStandardCapacity`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmissionLimits {
    wait: Duration,
    waiters: usize,
    /// Control reserve per class, per second, indexed by `RateClass::index`.
    reserve: [u32; RateClass::COUNT],
}

impl Default for AdmissionLimits {
    fn default() -> Self {
        let mut reserve = [0; RateClass::COUNT];
        reserve[RateClass::Standard.index()] = 2;
        Self {
            wait: Duration::from_secs(5),
            waiters: 256,
            reserve,
        }
    }
}

impl AdmissionLimits {
    /// Set the admission wait bound (`B-HTTP-06`).
    pub fn with_wait(mut self, wait: Duration) -> Result<Self, QuotaError> {
        if wait < Duration::from_millis(1) || wait > Duration::from_secs(60) {
            return Err(QuotaError::Bound("B-HTTP-06"));
        }
        self.wait = wait;
        Ok(self)
    }

    /// Set the waiter bound (`B-HTTP-07`).
    pub fn with_waiters(mut self, waiters: usize) -> Result<Self, QuotaError> {
        if !(1..=4096).contains(&waiters) {
            return Err(QuotaError::Bound("B-HTTP-07"));
        }
        self.waiters = waiters;
        Ok(self)
    }

    /// Set the control reserve of `class`, in requests per second
    /// (`B-HTTP-15`). Zero makes the two lanes equal in that class.
    ///
    /// ```
    /// use manja::kite::connect::admission::{AdmissionLimits, RateClass};
    ///
    /// let limits = AdmissionLimits::default()
    ///     .with_control_reserve(RateClass::Standard, 3)
    ///     .unwrap();
    /// assert_eq!(limits.control_reserve(RateClass::Standard), 3);
    /// assert!(AdmissionLimits::default().with_control_reserve(RateClass::Standard, 6).is_err());
    /// ```
    pub fn with_control_reserve(
        mut self,
        class: RateClass,
        per_second: u32,
    ) -> Result<Self, QuotaError> {
        if per_second > 5 {
            return Err(QuotaError::Bound("B-HTTP-15"));
        }
        self.reserve[class.index()] = per_second;
        Ok(self)
    }

    /// The admission wait bound.
    pub fn wait(&self) -> Duration {
        self.wait
    }

    /// The waiter bound.
    pub fn waiters(&self) -> usize {
        self.waiters
    }

    /// The control reserve of `class`, in requests per second.
    pub fn control_reserve(&self, class: RateClass) -> u32 {
        self.reserve[class.index()]
    }

    /// Slots of `window` reserved for the `Control` lane in `class`:
    /// the per-second reserve over the window's period, rounded up.
    fn reserved_slots(&self, class: RateClass, window: Window) -> u32 {
        let reserve = u128::from(self.control_reserve(class));
        let slots = (reserve * window.period.as_millis()).div_ceil(1000);
        u32::try_from(slots).unwrap_or(u32::MAX)
    }

    /// The lane's share of `window` in `class`: all of it for `Control`, the
    /// limit minus the reserved slots for `Standard`.
    fn lane_share(&self, class: RateClass, window: Window, priority: Priority) -> u32 {
        match priority {
            Priority::Control => window.limit,
            Priority::Standard => window
                .limit
                .saturating_sub(self.reserved_slots(class, window)),
        }
    }
}

/// A snapshot of one class's admission capacity, from [`Admission::headroom`].
///
/// Advisory and the local view: capacity can be taken between the read and
/// the call, and the scope cannot see other processes or the Kite app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Headroom {
    /// `Standard`-lane admissions the class would grant at this instant: its
    /// windows' Standard shares minus the live admissions, the smallest
    /// window deciding. 0 when the daily ceiling is spent.
    pub standard_now: u32,
    /// `Control`-lane admissions the class would grant at this instant: the
    /// whole window rate minus the live admissions. 0 when the daily
    /// ceiling is spent.
    pub control_now: u32,
    /// When `standard_now` is 0 because a window is full: the time until the
    /// `Standard` lane next has a slot. `None` when a slot is available now,
    /// when the daily ceiling is spent (no window frees it), and when the
    /// control reserve covers a window's rate, so the `Standard` lane can
    /// never be admitted in this class.
    pub next_slot_in: Option<Duration>,
    /// Placements left under the daily ceiling for `OrderPlacement`; `None`
    /// for every other class, which has no daily ceiling.
    pub day_remaining: Option<u32>,
}

/// Drop the entries of `log` older than `period`.
fn purge_expired(log: &mut VecDeque<(u64, Instant)>, now: Instant, period: Duration) {
    while log
        .front()
        .is_some_and(|(_, t)| now.duration_since(*t) >= period)
    {
        log.pop_front();
    }
}

/// When a lane whose share of the window is `share` next has a slot, or
/// `None` if it has one now: the lane has a slot once the log is below its
/// share, when the (len - share + 1)-th oldest entry ages out.
fn lane_frees_at(log: &VecDeque<(u64, Instant)>, share: u32, period: Duration) -> Option<Instant> {
    let share = share as usize;
    (log.len() >= share).then(|| log[log.len() - share].1 + period)
}

/// Why admission did not grant capacity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AdmissionError {
    /// The scope already has its maximum number of waiters.
    WaitersFull,
    /// Capacity did not become available within the wait bound.
    WaitExpired,
    /// The daily placement ceiling of the IST day is exhausted.
    DailyCeilingReached,
    /// The order's modification limit for the IST day is exhausted.
    ModificationLimitReached,
    /// The class's control reserve (`B-HTTP-15`) is at or above one of its
    /// window rates, so the `Standard` lane can never be admitted in it.
    NoStandardCapacity,
}

impl fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::WaitersFull => "the admission waiter bound is reached",
            Self::WaitExpired => "no admission capacity within the wait bound",
            Self::DailyCeilingReached => "the daily order ceiling is reached",
            Self::ModificationLimitReached => "the order's modification limit is reached",
            Self::NoStandardCapacity => {
                "the control reserve (B-HTTP-15) leaves the class no standard capacity"
            }
        })
    }
}

impl std::error::Error for AdmissionError {}

/// Source of wall-clock time for the IST day boundary. A test seam; the
/// default reads the system clock.
pub trait WallClock: Send + Sync + 'static {
    /// Seconds since the Unix epoch, UTC.
    fn unix_seconds(&self) -> i64;
}

struct SystemClock;

impl WallClock for SystemClock {
    fn unix_seconds(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default()
    }
}

#[derive(Default)]
struct DayState {
    day: i64,
    placements: u32,
    modifications: HashMap<String, u32>,
}

struct State {
    windows: HashMap<(RateClass, usize), VecDeque<(u64, Instant)>>,
    day: DayState,
    next_grant: u64,
}

struct Inner {
    profile: QuotaProfile,
    limits: AdmissionLimits,
    clock: Arc<dyn WallClock>,
    state: Mutex<State>,
    waiters: AtomicUsize,
}

/// A shared admission scope. Cheap to clone; clones share state.
#[derive(Clone)]
pub struct Admission(Arc<Inner>);

impl fmt::Debug for Admission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Admission")
            .field("profile", &self.0.profile.version)
            .field("limits", &self.0.limits)
            .field("waiters", &self.waiters())
            .finish()
    }
}

impl Default for Admission {
    fn default() -> Self {
        Self::new(QuotaProfile::kite_v3(), AdmissionLimits::default())
    }
}

/// Admitted capacity for one request.
///
/// Capacity is charged when the grant is issued. [`Self::consume`] marks it
/// as used by a dispatched request; dropping an unconsumed grant returns the
/// capacity to the scope, so an abandoned request costs nothing.
#[must_use = "dropping a grant without consuming it returns its capacity"]
pub struct AdmissionGrant {
    admission: Admission,
    class: RateClass,
    priority: Priority,
    id: u64,
    order_id: Option<String>,
    consumed: bool,
}

impl fmt::Debug for AdmissionGrant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdmissionGrant")
            .field("class", &self.class)
            .field("priority", &self.priority)
            .field("consumed", &self.consumed)
            .finish()
    }
}

impl AdmissionGrant {
    /// The rate class admitted.
    pub fn class(&self) -> RateClass {
        self.class
    }

    /// The lane admitted on.
    pub fn priority(&self) -> Priority {
        self.priority
    }

    /// Whether `self` was issued by `admission`.
    pub fn belongs_to(&self, admission: &Admission) -> bool {
        Arc::ptr_eq(&self.admission.0, &admission.0)
    }

    /// Mark the capacity as used by a dispatched request.
    pub fn consume(mut self) {
        self.consumed = true;
    }
}

impl Drop for AdmissionGrant {
    fn drop(&mut self) {
        if !self.consumed {
            self.admission
                .release(self.class, self.id, self.order_id.as_deref());
        }
    }
}

struct WaiterGuard<'a>(&'a AtomicUsize);

impl Drop for WaiterGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Admission {
    /// A new, independent budget scope.
    pub fn new(profile: QuotaProfile, limits: AdmissionLimits) -> Self {
        Self::with_clock(profile, limits, Arc::new(SystemClock))
    }

    /// A new scope reading the IST day from `clock` (test seam).
    pub fn with_clock(
        profile: QuotaProfile,
        limits: AdmissionLimits,
        clock: Arc<dyn WallClock>,
    ) -> Self {
        Self(Arc::new(Inner {
            profile,
            limits,
            clock,
            state: Mutex::new(State {
                windows: HashMap::new(),
                day: DayState::default(),
                next_grant: 0,
            }),
            waiters: AtomicUsize::new(0),
        }))
    }

    /// The profile in force.
    pub fn profile(&self) -> &QuotaProfile {
        &self.0.profile
    }

    /// The bounds in force.
    pub fn limits(&self) -> AdmissionLimits {
        self.0.limits
    }

    /// Operations currently waiting for capacity.
    pub fn waiters(&self) -> usize {
        self.0.waiters.load(Ordering::Acquire)
    }

    /// Whether two handles share one budget scope.
    pub fn same_scope(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    fn ist_day(&self) -> i64 {
        (self.0.clock.unix_seconds() + IST_OFFSET_SECONDS).div_euclid(86_400)
    }

    /// The state, locked, with the day counters reset when the IST day has
    /// changed since they were last touched.
    fn lock_today(&self) -> std::sync::MutexGuard<'_, State> {
        let today = self.ist_day();
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.day.day != today {
            state.day = DayState {
                day: today,
                ..Default::default()
            };
        }
        state
    }

    /// A snapshot of `class`'s capacity now: what each lane would be granted
    /// at this instant, when the `Standard` lane next has a slot if it has
    /// none, and the placements left under the daily ceiling.
    ///
    /// Advisory, and the local view: capacity can be taken between this
    /// read and the call that follows, and the scope cannot see other
    /// processes, other SDK instances or the Kite app, whose orders count
    /// against the same limits (`kite:exceptions.md:45-58`). Reading never
    /// reserves anything. The snapshot agrees with the next admission decision
    /// in this scope on the windows and the daily ceiling: if it reports `n`
    /// on a lane, `n` calls on that lane admit at once and the next one waits
    /// or is refused, subject for `OrderModification` to the order's own
    /// limit, which [`Self::modifications_remaining`] reads.
    ///
    /// ```
    /// use manja::kite::connect::admission::{Admission, RateClass};
    ///
    /// let admission = Admission::default();
    /// let h = admission.headroom(RateClass::Standard);
    /// // 10 per second, 2 of them reserved for the Control lane.
    /// assert_eq!((h.standard_now, h.control_now), (8, 10));
    /// assert_eq!(h.next_slot_in, None);
    /// assert_eq!(h.day_remaining, None);
    /// assert_eq!(admission.headroom(RateClass::OrderPlacement).day_remaining, Some(5000));
    /// ```
    pub fn headroom(&self, class: RateClass) -> Headroom {
        let now = Instant::now();
        let windows = self.0.profile.windows(class);
        let mut state = self.lock_today();
        let day_remaining = match class {
            RateClass::OrderPlacement => Some(
                self.0
                    .profile
                    .daily_order_ceiling
                    .saturating_sub(state.day.placements),
            ),
            _ => None,
        };
        let mut standard_now = u32::MAX;
        let mut control_now = u32::MAX;
        let mut standard_never = false;
        let mut next_slot: Option<Instant> = None;
        for (i, w) in windows.iter().enumerate() {
            let log = state.windows.entry((class, i)).or_default();
            purge_expired(log, now, w.period);
            let live = log.len() as u32;
            let control_share = self.0.limits.lane_share(class, *w, Priority::Control);
            let standard_share = self.0.limits.lane_share(class, *w, Priority::Standard);
            control_now = control_now.min(control_share.saturating_sub(live));
            if standard_share == 0 {
                standard_never = true;
                standard_now = 0;
                continue;
            }
            standard_now = standard_now.min(standard_share.saturating_sub(live));
            if let Some(frees) = lane_frees_at(log, standard_share, w.period) {
                next_slot = Some(next_slot.map_or(frees, |n: Instant| n.max(frees)));
            }
        }
        if day_remaining == Some(0) {
            // The ceiling refuses both lanes, and no window frees it.
            standard_now = 0;
            control_now = 0;
            next_slot = None;
        }
        let next_slot_in = if standard_now == 0 && !standard_never {
            next_slot.map(|t| t.saturating_duration_since(now))
        } else {
            None
        };
        Headroom {
            standard_now,
            control_now,
            next_slot_in,
            day_remaining,
        }
    }

    /// Modifications `order_id` may still make today under the profile's
    /// per-order limit, as this scope has counted them. Advisory and the
    /// local view, like [`Self::headroom`]: a modification sent by another
    /// process or the Kite app is not counted here.
    pub fn modifications_remaining(&self, order_id: &OrderId) -> u32 {
        let state = self.lock_today();
        let limit = self.0.profile.modifications_per_order;
        match state.day.modifications.get(order_id.as_str()) {
            Some(used) => limit.saturating_sub(*used),
            // An untracked order can only be tracked while the table, bounded
            // by the daily ceiling of orders, has room.
            None if state.day.modifications.len()
                >= self.0.profile.daily_order_ceiling as usize =>
            {
                0
            }
            None => limit,
        }
    }

    /// Wait at most `max_wait` (itself capped at the configured admission
    /// wait) for capacity in `class` on the `Standard` lane. `order_id`
    /// identifies the order of a modification, for the per-order limit.
    ///
    /// Dropping the returned future before it resolves abandons the wait and
    /// consumes nothing. [`Self::acquire_with_priority`] chooses the lane.
    pub async fn acquire(
        &self,
        class: RateClass,
        order_id: Option<&str>,
        max_wait: Duration,
    ) -> Result<AdmissionGrant, AdmissionError> {
        self.acquire_with_priority(Priority::Standard, class, order_id, max_wait)
            .await
    }

    /// [`Self::acquire`] on the lane `priority`: `Control` may use the
    /// class's whole rate, `Standard` the rate minus the control reserve
    /// (`B-HTTP-15`).
    pub async fn acquire_with_priority(
        &self,
        priority: Priority,
        class: RateClass,
        order_id: Option<&str>,
        max_wait: Duration,
    ) -> Result<AdmissionGrant, AdmissionError> {
        let deadline = Instant::now() + max_wait.min(self.0.limits.wait);
        let mut waiter: Option<WaiterGuard<'_>> = None;
        loop {
            match self.try_grant(priority, class, order_id)? {
                Ok(grant) => return Ok(grant),
                Err(ready_at) => {
                    if waiter.is_none() {
                        let prev = self.0.waiters.fetch_add(1, Ordering::AcqRel);
                        let guard = WaiterGuard(&self.0.waiters);
                        if prev >= self.0.limits.waiters {
                            drop(guard);
                            return Err(AdmissionError::WaitersFull);
                        }
                        waiter = Some(guard);
                    }
                    if ready_at > deadline {
                        return Err(AdmissionError::WaitExpired);
                    }
                    tokio::time::sleep_until(ready_at).await;
                }
            }
        }
    }

    // Ok(Ok(grant)) when admitted; Ok(Err(t)) when capacity frees at `t`;
    // Err(e) when admission is impossible for the day.
    #[allow(clippy::type_complexity)]
    fn try_grant(
        &self,
        priority: Priority,
        class: RateClass,
        order_id: Option<&str>,
    ) -> Result<Result<AdmissionGrant, Instant>, AdmissionError> {
        let now = Instant::now();
        let windows = self.0.profile.windows(class);
        let mut state = self.lock_today();
        match class {
            RateClass::OrderPlacement
                if state.day.placements >= self.0.profile.daily_order_ceiling =>
            {
                return Err(AdmissionError::DailyCeilingReached);
            }
            RateClass::OrderModification => {
                if let Some(id) = order_id {
                    let used = state.day.modifications.get(id).copied().unwrap_or(0);
                    let tracked = state.day.modifications.len();
                    let untracked = !state.day.modifications.contains_key(id);
                    // Tracking is bounded by the daily ceiling of orders.
                    if used >= self.0.profile.modifications_per_order
                        || (untracked && tracked >= self.0.profile.daily_order_ceiling as usize)
                    {
                        return Err(AdmissionError::ModificationLimitReached);
                    }
                }
            }
            _ => {}
        }
        let mut ready_at = None;
        for (i, w) in windows.iter().enumerate() {
            let share = self.0.limits.lane_share(class, *w, priority);
            if share == 0 {
                return Err(AdmissionError::NoStandardCapacity);
            }
            let log = state.windows.entry((class, i)).or_default();
            purge_expired(log, now, w.period);
            if let Some(frees) = lane_frees_at(log, share, w.period) {
                ready_at = Some(ready_at.map_or(frees, |r: Instant| r.max(frees)));
            }
        }
        if let Some(t) = ready_at {
            return Ok(Err(t));
        }
        state.next_grant += 1;
        let id = state.next_grant;
        for i in 0..windows.len() {
            state
                .windows
                .entry((class, i))
                .or_default()
                .push_back((id, now));
        }
        match class {
            RateClass::OrderPlacement => state.day.placements += 1,
            RateClass::OrderModification => {
                if let Some(id) = order_id {
                    *state.day.modifications.entry(id.to_string()).or_insert(0) += 1;
                }
            }
            _ => {}
        }
        Ok(Ok(AdmissionGrant {
            admission: self.clone(),
            class,
            priority,
            id,
            order_id: order_id.map(str::to_string),
            consumed: false,
        }))
    }

    fn release(&self, class: RateClass, id: u64, order_id: Option<&str>) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        for (key, log) in state.windows.iter_mut() {
            if key.0 == class {
                log.retain(|(g, _)| *g != id);
            }
        }
        match class {
            RateClass::OrderPlacement => {
                state.day.placements = state.day.placements.saturating_sub(1);
            }
            RateClass::OrderModification => {
                if let Some(count) = order_id.and_then(|o| state.day.modifications.get_mut(o)) {
                    *count = count.saturating_sub(1);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicI64;

    struct FakeClock(AtomicI64);

    impl WallClock for FakeClock {
        fn unix_seconds(&self) -> i64 {
            self.0.load(Ordering::Relaxed)
        }
    }

    // 2026-09-23 10:00:00 IST.
    const MORNING: i64 = 1_790_137_800;

    fn scope() -> (Admission, Arc<FakeClock>) {
        let clock = Arc::new(FakeClock(AtomicI64::new(MORNING)));
        let limits = AdmissionLimits::default()
            .with_wait(Duration::from_secs(60))
            .unwrap();
        (
            Admission::with_clock(QuotaProfile::kite_v3(), limits, clock.clone()),
            clock,
        )
    }

    const WAIT: Duration = Duration::from_secs(60);

    #[test]
    fn profile_validation() {
        let p = QuotaProfile::kite_v3();
        assert_eq!(p.version(), QUOTA_PROFILE_VERSION);
        let s = Duration::from_secs(1);
        let ok = |placement: Vec<Window>| {
            QuotaProfile::kite_v3().with_windows(RateClass::OrderPlacement, placement)
        };
        assert!(ok(vec![Window::new(10, s)]).is_ok());
        assert_eq!(
            ok(vec![Window::new(11, s)]),
            Err(QuotaError::PlacementAboveTenPerSecond)
        );
        assert_eq!(ok(vec![Window::new(0, s)]), Err(QuotaError::InvalidWindow));
        assert_eq!(
            ok(vec![Window::new(1, Duration::from_micros(10))]),
            Err(QuotaError::InvalidWindow)
        );
        assert_eq!(
            ok(vec![]),
            Err(QuotaError::MissingWindows(RateClass::OrderPlacement))
        );
        assert_eq!(
            QuotaProfile::kite_v3().with_windows(RateClass::Historical, vec![]),
            Err(QuotaError::MissingWindows(RateClass::Historical))
        );
        // Every class is set by name, so windows cannot land in the wrong one.
        let p = QuotaProfile::kite_v3()
            .with_version("t")
            .with_windows(RateClass::Historical, vec![Window::new(2, s)])
            .unwrap();
        assert_eq!(p.version(), "t");
        assert_eq!(p.windows(RateClass::Historical), vec![Window::new(2, s)]);
        assert_eq!(p.windows(RateClass::Quote), vec![Window::new(1, s)]);
        assert_eq!(
            QuotaProfile::kite_v3().with_windows(RateClass::Unclassified, vec![Window::new(1, s)]),
            Err(QuotaError::UnclassifiedWindows)
        );
        assert_eq!(
            QuotaProfile::kite_v3().with_daily_order_ceiling(0),
            Err(QuotaError::InvalidCeiling)
        );
        assert_eq!(
            QuotaProfile::kite_v3().with_modifications_per_order(0),
            Err(QuotaError::InvalidCeiling)
        );
    }

    #[test]
    fn unknown_endpoints_get_the_smallest_rate_not_unlimited() {
        let p = QuotaProfile::kite_v3();
        assert_eq!(
            RateClass::of(Method::Get, Endpoint::Unknown),
            RateClass::Unclassified
        );
        assert_eq!(
            p.windows(RateClass::Unclassified),
            vec![Window::new(1, Duration::from_secs(1))]
        );
        assert_eq!(
            RateClass::of(Method::Get, Endpoint::QuoteLtp),
            RateClass::Quote
        );
        assert_eq!(
            RateClass::of(Method::Delete, Endpoint::OrdersVarietyId),
            RateClass::Standard
        );
    }

    #[test]
    fn only_order_cancellation_and_gtt_deletion_default_to_control() {
        for method in Method::ALL {
            for endpoint in Endpoint::ALL {
                let expected = match (method, endpoint) {
                    (Method::Delete, Endpoint::OrdersVarietyId | Endpoint::GttTriggersId) => {
                        Priority::Control
                    }
                    _ => Priority::Standard,
                };
                assert_eq!(
                    Priority::default_for(*method, *endpoint),
                    expected,
                    "{method:?} {endpoint:?}"
                );
            }
        }
        assert_eq!(
            Priority::default_for(Method::Delete, Endpoint::SessionToken),
            Priority::Standard
        );
    }

    #[test]
    fn the_reserve_scales_with_the_window_period_rounding_up() {
        let limits = AdmissionLimits::default()
            .with_control_reserve(RateClass::OrderPlacement, 2)
            .unwrap();
        let slots = |limit, period| {
            limits.reserved_slots(RateClass::OrderPlacement, Window::new(limit, period))
        };
        assert_eq!(slots(10, Duration::from_secs(1)), 2);
        assert_eq!(slots(400, Duration::from_secs(60)), 120);
        assert_eq!(slots(5, Duration::from_millis(500)), 1);
        assert_eq!(slots(5, Duration::from_millis(1)), 1, "rounded up");
        assert_eq!(
            limits.reserved_slots(RateClass::Standard, Window::new(10, Duration::from_secs(1))),
            2,
            "the default standard reserve"
        );
        assert_eq!(
            limits.reserved_slots(RateClass::Quote, Window::new(1, Duration::from_secs(1))),
            0,
            "no reserve by default elsewhere"
        );
    }

    #[test]
    fn admission_bounds_are_validated() {
        let d = AdmissionLimits::default();
        assert_eq!((d.wait(), d.waiters()), (Duration::from_secs(5), 256));
        assert!(d.with_wait(Duration::from_millis(1)).is_ok());
        assert!(d.with_wait(Duration::from_secs(60)).is_ok());
        assert!(d.with_wait(Duration::from_micros(999)).is_err());
        assert!(d.with_wait(Duration::from_secs(61)).is_err());
        assert!(d.with_waiters(1).is_ok());
        assert!(d.with_waiters(4096).is_ok());
        assert!(d.with_waiters(0).is_err());
        assert!(d.with_waiters(4097).is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn quote_class_admits_one_per_second() {
        let (a, _) = scope();
        let start = Instant::now();
        a.acquire(RateClass::Quote, None, WAIT)
            .await
            .unwrap()
            .consume();
        a.acquire(RateClass::Quote, None, WAIT)
            .await
            .unwrap()
            .consume();
        assert_eq!(start.elapsed(), Duration::from_secs(1));
        // The standard class is independent.
        a.acquire(RateClass::Standard, None, WAIT)
            .await
            .unwrap()
            .consume();
        assert_eq!(start.elapsed(), Duration::from_secs(1));
    }

    #[tokio::test(start_paused = true)]
    async fn placement_honours_per_second_and_per_minute_windows() {
        let (a, _) = scope();
        let start = Instant::now();
        for _ in 0..10 {
            a.acquire(RateClass::OrderPlacement, None, WAIT)
                .await
                .unwrap()
                .consume();
        }
        assert_eq!(start.elapsed(), Duration::ZERO);
        a.acquire(RateClass::OrderPlacement, None, WAIT)
            .await
            .unwrap()
            .consume();
        assert_eq!(start.elapsed(), Duration::from_secs(1));
        // Fill the minute: 400 placements take 39 s at 10 per second, and
        // the 401st waits for the first minute's oldest grant to age out.
        for _ in 11..400 {
            a.acquire(RateClass::OrderPlacement, None, WAIT)
                .await
                .unwrap()
                .consume();
        }
        assert_eq!(start.elapsed(), Duration::from_secs(39));
        a.acquire(RateClass::OrderPlacement, None, WAIT)
            .await
            .unwrap()
            .consume();
        assert_eq!(start.elapsed(), Duration::from_secs(60));
    }

    #[tokio::test(start_paused = true)]
    async fn daily_ceiling_resets_at_the_ist_day_boundary() {
        let profile = QuotaProfile::kite_v3()
            .with_version("t")
            .with_daily_order_ceiling(3)
            .and_then(|p| p.with_modifications_per_order(2))
            .unwrap();
        let clock = Arc::new(FakeClock(AtomicI64::new(MORNING)));
        let a = Admission::with_clock(profile, AdmissionLimits::default(), clock.clone());
        for _ in 0..3 {
            a.acquire(RateClass::OrderPlacement, None, WAIT)
                .await
                .unwrap()
                .consume();
        }
        assert_eq!(
            a.acquire(RateClass::OrderPlacement, None, WAIT)
                .await
                .unwrap_err(),
            AdmissionError::DailyCeilingReached
        );
        // 23:59:59 IST is still the same day.
        clock.0.store(MORNING + 14 * 3600 - 1, Ordering::Relaxed);
        assert!(
            a.acquire(RateClass::OrderPlacement, None, WAIT)
                .await
                .is_err()
        );
        // 00:00:00 IST starts a new day.
        clock.0.store(MORNING + 14 * 3600, Ordering::Relaxed);
        a.acquire(RateClass::OrderPlacement, None, WAIT)
            .await
            .unwrap()
            .consume();
    }

    #[tokio::test(start_paused = true)]
    async fn modification_limit_is_per_order() {
        let (a, _) = scope();
        for _ in 0..25 {
            a.acquire(RateClass::OrderModification, Some("A"), WAIT)
                .await
                .unwrap()
                .consume();
        }
        assert_eq!(
            a.acquire(RateClass::OrderModification, Some("A"), WAIT)
                .await
                .unwrap_err(),
            AdmissionError::ModificationLimitReached
        );
        a.acquire(RateClass::OrderModification, Some("B"), WAIT)
            .await
            .unwrap()
            .consume();
    }

    #[tokio::test(start_paused = true)]
    async fn waiters_are_bounded_and_waits_are_bounded() {
        let clock = Arc::new(FakeClock(AtomicI64::new(MORNING)));
        let limits = AdmissionLimits::default()
            .with_waiters(1)
            .unwrap()
            .with_wait(Duration::from_millis(500))
            .unwrap();
        let a = Admission::with_clock(QuotaProfile::kite_v3(), limits, clock);
        a.acquire(RateClass::Quote, None, WAIT)
            .await
            .unwrap()
            .consume();
        // The next quote frees in 1 s, beyond the 500 ms wait bound.
        assert_eq!(
            a.acquire(RateClass::Quote, None, WAIT).await.unwrap_err(),
            AdmissionError::WaitExpired
        );
        assert_eq!(a.waiters(), 0);
        // With the bound raised, one waiter is admitted and a second is refused
        // at once rather than queued.
        let limits = AdmissionLimits::default().with_waiters(1).unwrap();
        let a = Admission::with_clock(
            QuotaProfile::kite_v3(),
            limits,
            Arc::new(FakeClock(AtomicI64::new(MORNING))),
        );
        a.acquire(RateClass::Quote, None, WAIT)
            .await
            .unwrap()
            .consume();
        let a2 = a.clone();
        let first = tokio::spawn(async move {
            a2.acquire(RateClass::Quote, None, WAIT)
                .await
                .map(|g| g.consume())
        });
        while a.waiters() == 0 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            a.acquire(RateClass::Quote, None, WAIT).await.unwrap_err(),
            AdmissionError::WaitersFull
        );
        first.await.unwrap().unwrap();
        assert_eq!(a.waiters(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn historical_candles_are_admitted_three_per_second() {
        let (a, _) = scope();
        let start = Instant::now();
        for _ in 0..3 {
            a.acquire(RateClass::Historical, None, WAIT)
                .await
                .unwrap()
                .consume();
        }
        assert_eq!(start.elapsed(), Duration::ZERO, "three fit in one second");
        // With the historical window full, a quote is still admitted at once.
        a.acquire(RateClass::Quote, None, WAIT)
            .await
            .unwrap()
            .consume();
        assert_eq!(
            start.elapsed(),
            Duration::ZERO,
            "quotes have their own window"
        );
        a.acquire(RateClass::Historical, None, WAIT)
            .await
            .unwrap()
            .consume();
        assert_eq!(start.elapsed(), Duration::from_secs(1), "the fourth waits");
        // And historical requests did not use the quote window: the next quote
        // (its window freed at 1 s) is admitted at once too.
        let before = Instant::now();
        a.acquire(RateClass::Quote, None, WAIT)
            .await
            .unwrap()
            .consume();
        assert_eq!(before.elapsed(), Duration::ZERO);
        assert_eq!(
            QuotaProfile::kite_v3().windows(RateClass::Historical),
            vec![Window::new(3, Duration::from_secs(1))]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cancelled_waits_and_abandoned_grants_return_capacity() {
        let (a, _) = scope();
        a.acquire(RateClass::Quote, None, WAIT)
            .await
            .unwrap()
            .consume();
        // A waiter dropped mid-wait leaves no waiter behind.
        let wait = a.acquire(RateClass::Quote, None, WAIT);
        let cancelled = tokio::time::timeout(Duration::from_millis(100), wait).await;
        assert!(cancelled.is_err());
        assert_eq!(a.waiters(), 0);
        // An abandoned grant is reclaimed: the next request is not delayed.
        tokio::time::advance(Duration::from_secs(1)).await;
        let grant = a.acquire(RateClass::Quote, None, WAIT).await.unwrap();
        drop(grant);
        let start = Instant::now();
        a.acquire(RateClass::Quote, None, WAIT)
            .await
            .unwrap()
            .consume();
        assert_eq!(start.elapsed(), Duration::ZERO);
        // The same holds for the daily ceiling.
        let g = a
            .acquire(RateClass::OrderPlacement, None, WAIT)
            .await
            .unwrap();
        drop(g);
        let state = a.0.state.lock().unwrap();
        assert_eq!(state.day.placements, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn clones_share_one_budget() {
        let (a, _) = scope();
        let b = a.clone();
        assert!(a.same_scope(&b));
        a.acquire(RateClass::Quote, None, WAIT)
            .await
            .unwrap()
            .consume();
        let start = Instant::now();
        b.acquire(RateClass::Quote, None, WAIT)
            .await
            .unwrap()
            .consume();
        assert_eq!(start.elapsed(), Duration::from_secs(1));
        let (independent, _) = scope();
        assert!(!a.same_scope(&independent));
    }
}
