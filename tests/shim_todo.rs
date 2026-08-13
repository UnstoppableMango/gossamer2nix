//! Placeholder call sites for the real shim (docs/deps/DESIGN.md §3.3).
//!
//! Each test is `#[ignore]`d and panics via `todo!()` by design — they exist
//! to mark where a future implementation should be wired in, not to pass.
//! Once a real patch function exists, each of these should be rewritten to
//! call it and assert its output against the corresponding `after/*.toml`
//! fixture (reusing the `common` helpers used by the other test files).

#[path = "common/mod.rs"]
mod common;

#[test]
#[ignore = "shim not implemented; see docs/deps/DESIGN.md §3.3"]
fn registry_dependency_gets_patched_by_real_shim() {
    todo!("call the manifest-patch shim on tests/fixtures/registry/before/ and diff against after/")
}

#[test]
#[ignore = "shim not implemented; see docs/deps/DESIGN.md §3.3"]
fn git_dependency_gets_patched_by_real_shim() {
    todo!("call the manifest-patch shim on tests/fixtures/git/before/ and diff against after/")
}

#[test]
#[ignore = "shim not implemented; see docs/deps/DESIGN.md §3.3"]
fn path_dependency_gets_repatched_by_real_shim() {
    todo!(
        "call the manifest-patch shim on tests/fixtures/path_passthrough/before/ and diff against after/"
    )
}

#[test]
#[ignore = "shim not implemented; see docs/deps/DESIGN.md §3.3"]
fn tarball_dependency_gets_patched_by_real_shim() {
    todo!("call the manifest-patch shim on tests/fixtures/tarball/before/ and diff against after/")
}

#[test]
#[ignore = "shim not implemented; see docs/deps/DESIGN.md §3.3"]
fn transitive_graph_gets_fully_patched_by_real_shim() {
    todo!(
        "call the manifest-patch shim on tests/fixtures/transitive/before/ (walking the full \
         project.lock entry list) and diff both patched manifests against after/"
    )
}
