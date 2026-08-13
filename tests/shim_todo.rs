//! Call sites for the real shim (docs/deps/DESIGN.md §3.3), invoked as a
//! subprocess against the compiled `gossamer2nix` binary:
//!
//! ```text
//! gossamer2nix generate --manifest-dir <before-dir> --out <out-dir>
//! ```
//!
//! `<before-dir>` is a `before/`-shaped fixture tree (DESIGN.md §3.1 reads
//! `project.toml`+`project.lock` together, hence both live in the same
//! `before/` dir); `<out-dir>` receives the patched manifest tree, mirroring
//! `<before-dir>`'s own relative structure 1:1 with the corresponding
//! `after/` fixture tree (single `project.toml` for the single-dep
//! scenarios; `root/project.toml` + `deps/linalg/project.toml` for the
//! transitive scenario).

#[path = "common/mod.rs"]
mod common;

use common::SINGLE_DEP_SCENARIOS;
use common::bin::{assert_manifest_matches, fresh_out_dir, run_generate_shim};
use std::path::Path;

#[test]
fn registry_dependency_gets_patched_by_real_shim() {
    run_single_dep_scenario("registry");
}

#[test]
fn git_dependency_gets_patched_by_real_shim() {
    run_single_dep_scenario("git");
}

#[test]
fn path_dependency_gets_repatched_by_real_shim() {
    run_single_dep_scenario("path_passthrough");
}

#[test]
fn tarball_dependency_gets_patched_by_real_shim() {
    run_single_dep_scenario("tarball");
}

#[test]
fn transitive_graph_gets_fully_patched_by_real_shim() {
    let out = fresh_out_dir("transitive");

    let output = run_generate_shim("tests/fixtures/transitive/before", &out);
    assert!(
        output.status.success(),
        "gossamer2nix generate failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_manifest_matches(
        out.join("root/project.toml"),
        "tests/fixtures/transitive/after/root/project.toml",
    );
    assert_manifest_matches(
        out.join("deps/linalg/project.toml"),
        "tests/fixtures/transitive/after/deps/linalg/project.toml",
    );
}

fn run_single_dep_scenario(name: &str) {
    let scenario = SINGLE_DEP_SCENARIOS
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no scenario named {name}"));
    let before_dir = Path::new(scenario.before_manifest)
        .parent()
        .expect("before_manifest must have a parent dir");
    let out = fresh_out_dir(scenario.name);

    let output = run_generate_shim(before_dir, &out);
    assert!(
        output.status.success(),
        "scenario {}: gossamer2nix generate failed: {}",
        scenario.name,
        String::from_utf8_lossy(&output.stderr)
    );

    assert_manifest_matches(out.join("project.toml"), scenario.after_manifest);
}
