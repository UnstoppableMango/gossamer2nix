//! The manifest-patching shim (docs/deps/DESIGN.md §3.3): rewrites every
//! dependency in a Gossamer project's transitive graph to a `{ path = ... }`
//! entry pointing at a store path, since `gos build` today only wires up
//! `path`-kind dependencies (docs/DEPS.md §12).
//!
//! This phase uses a fixed placeholder hash in place of a real Nix
//! fixed-output-derivation hash — real fetch/hash computation is
//! docs/deps/DESIGN.md §2/§3.2, a separate, later phase.

use crate::Lockfile;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const PLACEHOLDER_HASH: &str = "00000000000000000000000000000000";

#[derive(Debug, thiserror::Error)]
pub enum GenerateError {
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    TomlParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("serializing {path}: {source}")]
    TomlSerialize {
        path: PathBuf,
        #[source]
        source: toml::ser::Error,
    },
    #[error("no project.toml with a sibling project.lock found under {0}")]
    NoRootManifest(PathBuf),
    #[error("multiple project.toml files have a sibling project.lock under {0}: {1:?}")]
    AmbiguousRoot(PathBuf, Vec<PathBuf>),
    #[error("{manifest}: dependency {id:?} has no entry in root project.lock")]
    UnresolvedDependency { manifest: PathBuf, id: String },
}

pub fn generate(manifest_dir: &Path, out_dir: &Path) -> Result<(), GenerateError> {
    let manifests = find_manifests(manifest_dir)?;
    let root = find_root(manifest_dir, &manifests)?;

    let lock_path = root.with_file_name("project.lock");
    let lock_text = read_to_string(&lock_path)?;
    let lock: Lockfile = toml::from_str(&lock_text).map_err(|source| GenerateError::TomlParse {
        path: lock_path.clone(),
        source,
    })?;

    let store_paths = store_paths_from_lock(&lock);

    for manifest_path in &manifests {
        let patched = patch_manifest(manifest_path, &store_paths)?;
        let rel = manifest_path
            .strip_prefix(manifest_dir)
            .expect("manifest_path was collected from within manifest_dir");
        let dest = out_dir.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|source| GenerateError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        std::fs::write(&dest, patched)
            .map_err(|source| GenerateError::Io { path: dest, source })?;
    }

    Ok(())
}

fn store_paths_from_lock(lock: &Lockfile) -> BTreeMap<String, String> {
    lock.project
        .iter()
        .map(|entry| {
            let tail = entry.id.rsplit('/').next().unwrap_or(&entry.id);
            (
                entry.id.clone(),
                format!("/nix/store/{PLACEHOLDER_HASH}-{tail}"),
            )
        })
        .collect()
}

fn patch_manifest(
    manifest_path: &Path,
    store_paths: &BTreeMap<String, String>,
) -> Result<String, GenerateError> {
    let text = read_to_string(manifest_path)?;
    let mut value: toml::Value =
        toml::from_str(&text).map_err(|source| GenerateError::TomlParse {
            path: manifest_path.to_path_buf(),
            source,
        })?;

    if let Some(deps) = value
        .get_mut("dependencies")
        .and_then(toml::Value::as_table_mut)
    {
        for (id, dep_value) in deps.iter_mut() {
            let store_path =
                store_paths
                    .get(id)
                    .ok_or_else(|| GenerateError::UnresolvedDependency {
                        manifest: manifest_path.to_path_buf(),
                        id: id.clone(),
                    })?;
            let mut patched_table = toml::map::Map::new();
            patched_table.insert("path".to_string(), toml::Value::String(store_path.clone()));
            *dep_value = toml::Value::Table(patched_table);
        }
    }

    toml::to_string_pretty(&value).map_err(|source| GenerateError::TomlSerialize {
        path: manifest_path.to_path_buf(),
        source,
    })
}

fn find_root<'a>(
    manifest_dir: &Path,
    manifests: &'a [PathBuf],
) -> Result<&'a PathBuf, GenerateError> {
    let roots: Vec<&PathBuf> = manifests
        .iter()
        .filter(|m| m.with_file_name("project.lock").exists())
        .collect();

    match roots.as_slice() {
        [] => Err(GenerateError::NoRootManifest(manifest_dir.to_path_buf())),
        [root] => Ok(root),
        _ => Err(GenerateError::AmbiguousRoot(
            manifest_dir.to_path_buf(),
            roots.into_iter().cloned().collect(),
        )),
    }
}

fn find_manifests(manifest_dir: &Path) -> Result<Vec<PathBuf>, GenerateError> {
    let mut manifests = Vec::new();
    let mut stack = vec![manifest_dir.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|source| GenerateError::Io {
            path: dir.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| GenerateError::Io {
                path: dir.clone(),
                source,
            })?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|source| GenerateError::Io {
                path: path.clone(),
                source,
            })?;
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file()
                && path.file_name().and_then(|n| n.to_str()) == Some("project.toml")
            {
                manifests.push(path);
            }
        }
    }

    manifests.sort();
    Ok(manifests)
}

fn read_to_string(path: &Path) -> Result<String, GenerateError> {
    std::fs::read_to_string(path).map_err(|source| GenerateError::Io {
        path: path.to_path_buf(),
        source,
    })
}
