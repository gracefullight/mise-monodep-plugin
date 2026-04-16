use crate::models::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub fn find_workspace_root(start: &Path) -> Result<PathBuf> {
    let current = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    let mut candidate = Some(current.as_path());
    while let Some(dir) = candidate {
        let config = dir.join("mise.toml");
        if config.exists() {
            let content = std::fs::read_to_string(&config)?;
            let parsed: toml::Value = toml::from_str(&content)?;
            if parsed
                .get("monorepo")
                .and_then(|m| m.get("config_roots"))
                .and_then(|c| c.as_array())
                .is_some_and(|roots| !roots.is_empty())
            {
                return Ok(dir.to_path_buf());
            }
        }
        candidate = dir.parent();
    }
    Err(MonodepError::General(
        "Unable to find a mise monorepo root with [monorepo].config_roots".into(),
    ))
}

pub fn discover_workspaces(root: &Path) -> Result<BTreeMap<String, WorkspacePackage>> {
    let config_path = root.join("mise.toml");
    let content = std::fs::read_to_string(&config_path)?;
    let parsed: toml::Value = toml::from_str(&content)?;

    let patterns = parsed
        .get("monorepo")
        .and_then(|m| m.get("config_roots"))
        .and_then(|c| c.as_array())
        .ok_or_else(|| {
            MonodepError::General(
                "No [monorepo].config_roots entries were found in mise.toml".into(),
            )
        })?;

    let mut workspaces = BTreeMap::new();
    for pattern in patterns {
        let pattern_str = pattern.as_str().unwrap_or("");
        for entry in glob::glob(&root.join(pattern_str).to_string_lossy())
            .map_err(|e| MonodepError::General(format!("Invalid glob pattern: {e}")))?
        {
            let candidate = match entry {
                Ok(p) => p,
                Err(_) => continue,
            };
            let manifest_path = candidate.join("package.json");
            if !manifest_path.exists() {
                // Check for pyproject.toml (Python workspace without package.json)
                if candidate.join("pyproject.toml").exists() {
                    let name = candidate
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    workspaces.insert(
                        name.clone(),
                        WorkspacePackage {
                            name,
                            path: candidate.canonicalize().unwrap_or(candidate),
                            manifest: serde_json::Value::Null,
                        },
                    );
                }
                continue;
            }
            let manifest: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&manifest_path)?)?;
            let name = manifest
                .get("name")
                .and_then(|n| n.as_str())
                .ok_or_else(|| {
                    MonodepError::General(format!(
                        "Missing package name in {}",
                        manifest_path.display()
                    ))
                })?
                .to_string();
            workspaces.insert(
                name.clone(),
                WorkspacePackage {
                    name,
                    path: candidate.canonicalize().unwrap_or(candidate),
                    manifest,
                },
            );
        }
    }

    if workspaces.is_empty() {
        return Err(MonodepError::General(
            "No workspace package.json files were discovered from monorepo.config_roots".into(),
        ));
    }
    Ok(workspaces)
}

pub fn dependency_entries(
    manifest: &serde_json::Value,
    options: &SyncOptions,
) -> Vec<ManifestDependency> {
    let mut entries = Vec::new();

    let mut merge = |group: &str, optional: bool| {
        if let Some(deps) = manifest.get(group).and_then(|d| d.as_object()) {
            for (name, spec) in deps {
                entries.push(ManifestDependency {
                    name: name.clone(),
                    spec: spec.as_str().unwrap_or("").to_string(),
                    group: group.to_string(),
                    optional,
                });
            }
        }
    };

    merge("dependencies", false);
    if options.include_dev {
        merge("devDependencies", false);
    }
    if options.include_optional {
        merge("optionalDependencies", true);
    }
    entries
}

pub fn closure_for_filters(
    workspaces: &BTreeMap<String, WorkspacePackage>,
    filters: &[String],
    options: &SyncOptions,
) -> Result<BTreeMap<String, WorkspacePackage>> {
    if filters.is_empty() {
        return Ok(workspaces.clone());
    }

    let mut selected = BTreeMap::new();
    let mut pending: Vec<String> = filters.to_vec();

    while let Some(name) = pending.pop() {
        if selected.contains_key(&name) {
            continue;
        }
        let ws = workspaces
            .get(&name)
            .ok_or_else(|| MonodepError::General(format!("Unknown workspace filter '{name}'")))?;
        selected.insert(name.clone(), ws.clone());
        for dep in dependency_entries(&ws.manifest, options) {
            if dep.spec.starts_with("workspace:") && workspaces.contains_key(&dep.name) {
                pending.push(dep.name);
            }
        }
    }
    Ok(selected)
}

pub fn bin_entries(name: &str, manifest: &serde_json::Value) -> BTreeMap<String, String> {
    let mut bins = BTreeMap::new();
    match manifest.get("bin") {
        Some(serde_json::Value::String(s)) => {
            bins.insert(name.to_string(), s.clone());
        }
        Some(serde_json::Value::Object(obj)) => {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    bins.insert(k.clone(), s.to_string());
                }
            }
        }
        _ => {}
    }
    bins
}
