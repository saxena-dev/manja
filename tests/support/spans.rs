// Thread-local span capture that other tests in the same binary cannot
// switch off.
//
// `tracing` caches each callsite's interest globally. While at most one
// dispatcher has been created, `tracing-core` computes that interest from
// the default of whichever thread registers the callsite first. So if a
// test without a subscriber reaches a span first, the span is cached as
// never enabled, and a concurrent test's capture layer then sees none of
// it. A second dispatcher that lives for the whole binary makes every
// registration combine all live dispatchers instead.

use std::sync::OnceLock;

use tracing::subscriber::{DefaultGuard, NoSubscriber};
use tracing::{Dispatch, Subscriber};

/// Set `subscriber` as this thread's default until the guard drops.
pub fn set_default<S: Subscriber + Send + Sync + 'static>(subscriber: S) -> DefaultGuard {
    static KEEP: OnceLock<Dispatch> = OnceLock::new();
    KEEP.get_or_init(|| Dispatch::new(NoSubscriber::default()));
    tracing::subscriber::set_default(subscriber)
}
