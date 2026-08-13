//! Structural spec for the manifest-patching shim (docs/deps/DESIGN.md §3.3).
//!
//! No implementation exists yet, so these tests can't call real code. They
//! instead assert that the hand-authored `after/` fixtures under
//! `tests/fixtures/` are internally consistent with the invariants the shim
//! must satisfy once built: dependency ids are preserved, every dependency
//! becomes a single-key absolute `path` table, and store paths are unique
//! per dependency id.

#[path = "common/mod.rs"]
mod common;

use common::{SINGLE_DEP_SCENARIOS, as_single_key_path, dep_ids, load_manifest};
use std::collections::HashSet;

#[test]
fn dependency_ids_are_preserved_across_the_patch() {
    for scenario in SINGLE_DEP_SCENARIOS {
        let before = load_manifest(scenario.before_manifest);
        let after = load_manifest(scenario.after_manifest);
        assert_eq!(
            dep_ids(&before),
            dep_ids(&after),
            "scenario {}: shim must not add or remove dependencies",
            scenario.name
        );
    }
}

#[test]
fn every_patched_dependency_is_a_single_key_absolute_path_table() {
    for scenario in SINGLE_DEP_SCENARIOS {
        let after = load_manifest(scenario.after_manifest);
        for (id, value) in &after.dependencies {
            let path = as_single_key_path(value).unwrap_or_else(|| {
                panic!(
                    "scenario {}: dependency {id} is not a single-key `path` table: {value:?}",
                    scenario.name
                )
            });
            assert!(
                path.starts_with('/'),
                "scenario {}: patched path for {id} must be absolute, got {path:?}",
                scenario.name
            );
        }
    }
}

#[test]
fn fetched_kinds_change_shape_not_just_value() {
    for scenario in SINGLE_DEP_SCENARIOS {
        if scenario.name == "path_passthrough" {
            continue;
        }
        let before = load_manifest(scenario.before_manifest);
        for value in before.dependencies.values() {
            assert!(
                as_single_key_path(value).is_none(),
                "scenario {}: before-fixture dependency should not already look like \
                 a patched single-key path table (fixture bug?)",
                scenario.name
            );
        }
    }
}

#[test]
fn path_passthrough_keeps_kind_but_rewrites_value_to_absolute() {
    let scenario = SINGLE_DEP_SCENARIOS
        .iter()
        .find(|s| s.name == "path_passthrough")
        .unwrap();
    let before = load_manifest(scenario.before_manifest);
    let after = load_manifest(scenario.after_manifest);

    let (id, before_value) = before.dependencies.iter().next().unwrap();
    let before_path = as_single_key_path(before_value)
        .expect("path_passthrough before-fixture must already be a `path` table");
    assert!(
        !before_path.starts_with('/'),
        "before-fixture path should be relative, got {before_path:?}"
    );

    let after_path = as_single_key_path(&after.dependencies[id])
        .expect("after-fixture must be a single-key path table");
    assert!(
        after_path.starts_with('/'),
        "after-fixture path should be absolute, got {after_path:?}"
    );
    assert_ne!(
        before_path, after_path,
        "shim must rewrite the path value even when the dependency was already `path`-kind"
    );
}

#[test]
fn transitive_graph_patches_every_manifest_with_distinct_store_paths() {
    let root_before = load_manifest("tests/fixtures/transitive/before/root/project.toml");
    let root_after = load_manifest("tests/fixtures/transitive/after/root/project.toml");
    let linalg_before = load_manifest("tests/fixtures/transitive/before/deps/linalg/project.toml");
    let linalg_after = load_manifest("tests/fixtures/transitive/after/deps/linalg/project.toml");

    assert_eq!(dep_ids(&root_before), dep_ids(&root_after));
    assert_eq!(dep_ids(&linalg_before), dep_ids(&linalg_after));

    let mut seen_paths = HashSet::new();
    for manifest in [&root_after, &linalg_after] {
        for (id, value) in &manifest.dependencies {
            let path = as_single_key_path(value)
                .unwrap_or_else(|| panic!("dependency {id} not patched to a path table"));
            assert!(
                seen_paths.insert(path.to_string()),
                "store path {path:?} reused across different dependency ids"
            );
        }
    }
    assert_eq!(
        seen_paths.len(),
        2,
        "expected one distinct store path each for linalg (via root) and blas (via linalg)"
    );
}
