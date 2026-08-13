//! The manifest-patching shim (docs/deps/DESIGN.md §3.3) relies on one
//! stdlib fact: joining a relative base with an absolute path discards the
//! base entirely. That's why writing `{ path = "<absolute nix store path>" }"`
//! into a manifest works with zero special-casing on Gossamer's side, no
//! matter where that manifest's containing directory happens to sit.

use std::path::{Path, PathBuf};

#[test]
fn absolute_join_discards_the_base() {
    let manifest_dir = Path::new("/some/manifest/dir");
    let store_path = Path::new("/nix/store/00000000000000000000000000000000-linalg");

    assert_eq!(
        manifest_dir.join(store_path),
        PathBuf::from("/nix/store/00000000000000000000000000000000-linalg"),
        "an absolute RHS must fully replace the base, which is the mechanism \
         the shim depends on"
    );
}

#[test]
fn relative_join_would_not_have_worked() {
    let manifest_dir = Path::new("/some/manifest/dir");
    let relative = Path::new("../internal");

    assert_ne!(
        manifest_dir.join(relative),
        PathBuf::from("../internal"),
        "a relative RHS is appended to the base instead of replacing it, which is \
         exactly why the shim must rewrite paths to be absolute"
    );
}
