//! Shared fixture-parsing helpers for the shim spec tests.
//!
//! `Manifest`/`Lockfile` live in the `gossamer2nix` lib crate so tests and
//! the future shim implementation deserialize against the same types.
//! Keeping helpers parameterized over a parsed `Manifest`/`Lockfile` (not a
//! fixture path) means a future test can feed a real shim's output through
//! these same assertions with minimal rewrite.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

pub use gossamer2nix::{DependencySpec, LockEntry, LockSource, Lockfile, Manifest, ProjectMeta};

pub mod bin;

pub fn load_manifest(path: impl AsRef<Path>) -> Manifest {
    let text = fs::read_to_string(path.as_ref())
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.as_ref().display()));
    toml::from_str(&text).unwrap_or_else(|e| panic!("parsing {}: {e}", path.as_ref().display()))
}

pub fn load_lockfile(path: impl AsRef<Path>) -> Lockfile {
    let text = fs::read_to_string(path.as_ref())
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.as_ref().display()));
    toml::from_str(&text).unwrap_or_else(|e| panic!("parsing {}: {e}", path.as_ref().display()))
}

pub fn dep_ids(m: &Manifest) -> BTreeSet<String> {
    m.dependencies.keys().cloned().collect()
}

/// `Some(path)` only if `value` is `DependencySpec::Path`. This is the shape
/// every dependency entry must take after the shim patches a manifest.
pub fn as_single_key_path(value: &DependencySpec) -> Option<&str> {
    match value {
        DependencySpec::Path { path } => Some(path.as_str()),
        _ => None,
    }
}

pub fn lock_entry<'a>(lock: &'a Lockfile, id: &str) -> &'a LockEntry {
    lock.project
        .iter()
        .find(|entry| entry.id == id)
        .unwrap_or_else(|| panic!("no lock entry for id {id}"))
}

pub fn lock_entry_source(lock: &Lockfile, id: &str) -> &'static str {
    lock_entry(lock, id).source.kind()
}

pub struct Scenario {
    pub name: &'static str,
    pub before_manifest: &'static str,
    pub before_lock: &'static str,
    pub after_manifest: &'static str,
}

pub const SINGLE_DEP_SCENARIOS: &[Scenario] = &[
    Scenario {
        name: "registry",
        before_manifest: "tests/fixtures/registry/before/project.toml",
        before_lock: "tests/fixtures/registry/before/project.lock",
        after_manifest: "tests/fixtures/registry/after/project.toml",
    },
    Scenario {
        name: "git",
        before_manifest: "tests/fixtures/git/before/project.toml",
        before_lock: "tests/fixtures/git/before/project.lock",
        after_manifest: "tests/fixtures/git/after/project.toml",
    },
    Scenario {
        name: "path_passthrough",
        before_manifest: "tests/fixtures/path_passthrough/before/project.toml",
        before_lock: "tests/fixtures/path_passthrough/before/project.lock",
        after_manifest: "tests/fixtures/path_passthrough/after/project.toml",
    },
    Scenario {
        name: "tarball",
        before_manifest: "tests/fixtures/tarball/before/project.toml",
        before_lock: "tests/fixtures/tarball/before/project.lock",
        after_manifest: "tests/fixtures/tarball/after/project.toml",
    },
];
