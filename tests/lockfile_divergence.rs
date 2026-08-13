//! Demonstrates why `--locked` must never be passed to `gos build` after the
//! shim runs (docs/deps/DESIGN.md §3.3, step 4): the patched manifest no
//! longer matches `project.lock`'s recorded source kind/fields for any
//! dependency, fetched or not. These tests assert that divergence holds
//! across every fixture scenario, as a structural proxy for that design
//! decision.

#[path = "common/mod.rs"]
mod common;

use common::{
    SINGLE_DEP_SCENARIOS, as_single_key_path, load_lockfile, load_manifest, lock_entry_source,
};

#[test]
fn patched_manifest_diverges_from_lock_recorded_source() {
    for scenario in SINGLE_DEP_SCENARIOS {
        if scenario.name == "path_passthrough" {
            // path-kind divergence is covered by
            // path_passthrough_lock_path_differs_from_patched_path below.
            continue;
        }

        let lock = load_lockfile(scenario.before_lock);
        let after = load_manifest(scenario.after_manifest);

        let (id, after_value) = after.dependencies.iter().next().unwrap();
        let source = lock_entry_source(&lock, id);
        let after_path = as_single_key_path(after_value)
            .unwrap_or_else(|| panic!("scenario {}: expected patched path table", scenario.name));

        assert_ne!(
            source, "path",
            "scenario {}: expected a fetched (non-path) source kind",
            scenario.name
        );
        assert!(
            after_path.starts_with("/nix/store/"),
            "scenario {}: patched path should be a store path, not the lock's own {source} \
             fields, which is exactly why re-checking against project.lock (--locked) would fail",
            scenario.name
        );
    }
}

#[test]
fn path_passthrough_lock_path_differs_from_patched_path() {
    let scenario = SINGLE_DEP_SCENARIOS
        .iter()
        .find(|s| s.name == "path_passthrough")
        .unwrap();
    let lock = load_lockfile(scenario.before_lock);
    let after = load_manifest(scenario.after_manifest);

    let (id, after_value) = after.dependencies.iter().next().unwrap();
    assert_eq!(lock_entry_source(&lock, id), "path");

    let lock_path = match &common::lock_entry(&lock, id).source {
        common::LockSource::Path { path } => path.as_str(),
        other => panic!("path-kind lock entry must record its own `path` field, got {other:?}"),
    };
    let after_path = as_single_key_path(after_value).expect("expected patched path table");

    assert_ne!(
        lock_path, after_path,
        "even for an originally path-kind dep, the lock's recorded path and the shim's \
         patched path must differ, otherwise --locked would trivially still work here"
    );
}
