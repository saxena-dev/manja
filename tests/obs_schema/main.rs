//! Schema snapshot for the frozen observability catalogue
//! (`OBS_SCHEMA_VERSION` 1): instrument names, types, units, exact label
//! sets and series bounds; span names, levels and fields; and every closed
//! label domain. A change to `catalogue.v1.txt` is a schema change and needs
//! the compatibility treatment the schema module documents.

use manja::kite::obs::schema::{catalogue_text, OBS_SCHEMA_VERSION};

#[test]
fn catalogue_matches_the_committed_snapshot() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/obs_schema/catalogue.v1.txt"
    );
    let expected = std::fs::read_to_string(path).unwrap();
    let actual = catalogue_text();
    assert_eq!(OBS_SCHEMA_VERSION, 1);
    if actual != expected {
        for (i, (a, e)) in actual.lines().zip(expected.lines()).enumerate() {
            if a != e {
                panic!("line {}:\n  actual   {a}\n  expected {e}", i + 1);
            }
        }
        panic!("catalogue length differs from {path}");
    }
}

#[test]
fn observability_works_without_any_runtime_or_collector() {
    // A plain #[test] has no async runtime, so nothing here could spawn a
    // task; construction also opens no listener and installs no global.
    let obs = manja::kite::obs::Observability::disabled();
    assert!(!obs.is_recording());
    assert!(obs.collect_gauges().is_empty());
    // Nothing in this binary installs a global subscriber, and constructing
    // the handle must not either.
    assert!(!tracing::dispatcher::has_been_set());
}
