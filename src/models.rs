use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum MonodepError {
    #[error("{0}")]
    General(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("TOML error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("Walk error: {0}")]
    Walk(#[from] walkdir::Error),
}

pub type Result<T> = std::result::Result<T, MonodepError>;

#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncOptions {
    pub include_dev: bool,
    pub include_optional: bool,
    pub skip_install: bool,
}

impl SyncOptions {
    pub fn new() -> Self {
        Self {
            include_dev: true,
            include_optional: true,
            skip_install: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ManifestDependency {
    pub name: String,
    pub spec: String,
    pub group: String,
    pub optional: bool,
}

#[derive(Debug, Clone)]
pub struct WorkspacePackage {
    #[allow(dead_code)]
    pub name: String,
    pub path: PathBuf,
    pub manifest: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedDependency {
    pub kind: String,
    pub name: String,
    pub spec: String,
    pub group: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DedupStats {
    pub deduplicated_packages: usize,
    pub duplicate_copies_saved: usize,
    pub files_hardlinked: usize,
    pub packages_scanned: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DedupResult {
    pub node: DedupStats,
    pub python: DedupStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SyncPlan {
    pub options: SyncOptions,
    pub package_manager: String,
    pub root: PathBuf,
    pub selected_workspaces: Vec<String>,
    pub workspace_links: BTreeMap<String, BTreeMap<String, ResolvedDependency>>,
    pub workspaces: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dedup: Option<DedupResult>,
}
