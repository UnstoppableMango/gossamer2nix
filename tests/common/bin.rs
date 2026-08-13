//! Compiled-binary invocation helpers for `tests/shim_todo.rs`.
//!
//! Separate from `mod.rs` (fixture-parsing) since this drives the
//! `gossamer2nix` binary as a subprocess rather than parsing TOML in-process.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Invokes the compiled `gossamer2nix` binary's (proposed, unimplemented)
/// `generate` subcommand against `manifest_dir`, writing the patched
/// manifest tree to `out_dir`. See `tests/shim_todo.rs` for the assumed
/// contract (DESIGN.md §3.1/§3.3).
pub fn run_generate_shim(manifest_dir: impl AsRef<Path>, out_dir: impl AsRef<Path>) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gossamer2nix"))
        .arg("generate")
        .arg("--manifest-dir")
        .arg(manifest_dir.as_ref())
        .arg("--out")
        .arg(out_dir.as_ref())
        .output()
        .expect("failed to execute gossamer2nix binary")
}

/// Fresh, empty temp directory for a single test's `generate` output, named
/// after the scenario so parallel test runs don't collide.
pub fn fresh_out_dir(scenario: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gossamer2nix-shim-test-{scenario}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("creating {}: {e}", dir.display()));
    dir
}

/// Asserts the manifest written at `actual_path` is exactly equal (full
/// `toml::Value`, including `[project]`) to the hand-authored fixture at
/// `expected_path`.
pub fn assert_manifest_matches(actual_path: impl AsRef<Path>, expected_path: impl AsRef<Path>) {
    let actual_text = fs::read_to_string(actual_path.as_ref())
        .unwrap_or_else(|e| panic!("reading {}: {e}", actual_path.as_ref().display()));
    let expected_text = fs::read_to_string(expected_path.as_ref())
        .unwrap_or_else(|e| panic!("reading {}: {e}", expected_path.as_ref().display()));

    let actual: toml::Value = toml::from_str(&actual_text)
        .unwrap_or_else(|e| panic!("parsing {}: {e}", actual_path.as_ref().display()));
    let expected: toml::Value = toml::from_str(&expected_text)
        .unwrap_or_else(|e| panic!("parsing {}: {e}", expected_path.as_ref().display()));

    assert_eq!(
        actual,
        expected,
        "{} does not match {}",
        actual_path.as_ref().display(),
        expected_path.as_ref().display()
    );
}
