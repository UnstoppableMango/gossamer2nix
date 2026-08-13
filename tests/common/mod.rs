//! Shared fixture-parsing helpers for the shim spec tests.
//!
//! No real `DependencySpec` type exists yet (the shim isn't implemented),
//! so dependency values are parsed as untyped `toml::Value` rather than a
//! typed enum. Keeping helpers parameterized over a parsed `Manifest`/
//! `Lockfile` (not a fixture path) means a future test can feed a real
//! shim's output through these same assertions with minimal rewrite.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

#[derive(Debug, serde::Deserialize)]
pub struct Manifest {
    #[allow(dead_code)]
    pub project: toml::Value,
    pub dependencies: std::collections::BTreeMap<String, toml::Value>,
}

#[derive(Debug, serde::Deserialize)]
pub struct Lockfile {
    pub project: Vec<toml::Value>,
}

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

/// `Some(path)` only if `value` is a table with exactly one key, `"path"`,
/// whose value is a string. This is the shape every dependency entry must
/// take after the shim patches a manifest.
pub fn as_single_key_path(value: &toml::Value) -> Option<&str> {
    let table = value.as_table()?;
    if table.len() != 1 {
        return None;
    }
    table.get("path")?.as_str()
}

pub fn lock_entry<'a>(lock: &'a Lockfile, id: &str) -> &'a toml::Value {
    lock.project
        .iter()
        .find(|entry| entry.get("id").and_then(|v| v.as_str()) == Some(id))
        .unwrap_or_else(|| panic!("no lock entry for id {id}"))
}

pub fn lock_entry_source<'a>(lock: &'a Lockfile, id: &str) -> &'a str {
    lock_entry(lock, id)
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("lock entry {id} missing `source`"))
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
