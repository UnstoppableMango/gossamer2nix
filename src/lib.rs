#[derive(Debug, serde::Deserialize)]
pub struct Manifest {
    pub project: ProjectMeta,
    pub dependencies: std::collections::BTreeMap<String, DependencySpec>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ProjectMeta {
    pub id: String,
    pub version: String,
    #[serde(default = "default_edition")]
    pub edition: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub license: String,
    pub output: Option<String>,
    pub entry: Option<String>,
}

fn default_edition() -> String {
    "2026".to_string()
}

#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
pub enum DependencySpec {
    Registry(String),
    Git {
        git: String,
        tag: Option<String>,
        branch: Option<String>,
        rev: Option<String>,
    },
    Tarball {
        url: String,
        sha256: String,
    },
    Path {
        path: String,
    },
}

#[derive(Debug, serde::Deserialize)]
pub struct Lockfile {
    pub project: Vec<LockEntry>,
}

#[derive(Debug, serde::Deserialize)]
pub struct LockEntry {
    pub id: String,
    #[serde(flatten)]
    pub source: LockSource,
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "source", rename_all = "lowercase")]
pub enum LockSource {
    Registry {
        version: String,
        sha256: String,
        owner_pubkey: Option<String>,
    },
    Git {
        url: String,
        #[serde(rename = "ref")]
        reference: String,
        sha256: String,
    },
    Path {
        path: String,
    },
    Tarball {
        url: String,
        tarball_sha256: String,
    },
}

impl LockSource {
    pub fn kind(&self) -> &'static str {
        match self {
            LockSource::Registry { .. } => "registry",
            LockSource::Git { .. } => "git",
            LockSource::Path { .. } => "path",
            LockSource::Tarball { .. } => "tarball",
        }
    }
}
