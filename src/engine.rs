use crate::dedup::{deduplicate_workspaces, prune_store};
use crate::manifest::{
    bin_entries, closure_for_filters, dependency_entries, discover_workspaces,
};
use crate::models::*;
use crate::pm::{
    detect_package_manager, run_add, run_install, run_remove, run_update, run_uv_sync,
    workspace_has_dependency,
};
use std::collections::{BTreeMap, HashSet};
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};

const MANIFEST_MARKER: &str = ".monodep-managed.json";

fn relative_symlink(target: &Path, link_path: &Path) -> Result<()> {
    if let Some(parent) = link_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if link_path.exists() || link_path.symlink_metadata().is_ok() {
        if link_path.is_dir() && !link_path.symlink_metadata()?.file_type().is_symlink() {
            std::fs::remove_dir_all(link_path)?;
        } else {
            std::fs::remove_file(link_path)?;
        }
    }
    let parent = link_path.parent().unwrap_or(Path::new("."));
    let relative = pathdiff::diff_paths(target, parent).unwrap_or_else(|| target.to_path_buf());
    unix_fs::symlink(&relative, link_path)?;
    Ok(())
}

fn remove_managed_links(node_modules: &Path) -> Result<()> {
    let marker = node_modules.join(MANIFEST_MARKER);
    if !marker.exists() {
        return Ok(());
    }
    let data: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&marker)?)?;

    if let Some(deps) = data.get("dependencies").and_then(|d| d.as_array()) {
        for dep in deps {
            if let Some(name) = dep.as_str() {
                let path = node_modules.join(name);
                if path.exists() || path.symlink_metadata().is_ok() {
                    if path.is_dir() && !path.symlink_metadata()?.file_type().is_symlink() {
                        std::fs::remove_dir_all(&path)?;
                    } else {
                        std::fs::remove_file(&path)?;
                    }
                }
            }
        }
    }

    let bin_dir = node_modules.join(".bin");
    if let Some(bins) = data.get("bins").and_then(|b| b.as_array()) {
        for bin in bins {
            if let Some(name) = bin.as_str() {
                let path = bin_dir.join(name);
                if path.exists() || path.symlink_metadata().is_ok() {
                    std::fs::remove_file(&path)?;
                }
            }
        }
        if bin_dir.exists() && std::fs::read_dir(&bin_dir)?.next().is_none() {
            std::fs::remove_dir(&bin_dir)?;
        }
    }

    std::fs::remove_file(&marker)?;
    Ok(())
}

fn write_managed_marker(
    node_modules: &Path,
    dependencies: &[String],
    bins: &[String],
) -> Result<()> {
    std::fs::create_dir_all(node_modules)?;
    let mut deps = dependencies.to_vec();
    deps.sort();
    let mut bin_list = bins.to_vec();
    bin_list.sort();
    let payload = serde_json::json!({
        "bins": bin_list,
        "dependencies": deps,
    });
    std::fs::write(
        node_modules.join(MANIFEST_MARKER),
        serde_json::to_string_pretty(&payload)? + "\n",
    )?;
    Ok(())
}

fn resolve_local_deps(
    workspace: &WorkspacePackage,
    workspaces: &BTreeMap<String, WorkspacePackage>,
    options: &SyncOptions,
) -> Result<BTreeMap<String, ResolvedDependency>> {
    let mut links = BTreeMap::new();
    for dep in dependency_entries(&workspace.manifest, options) {
        if dep.spec.starts_with("workspace:") {
            let target_ws = workspaces.get(&dep.name).ok_or_else(|| {
                MonodepError::General(format!(
                    "Workspace dependency '{}' was not discovered",
                    dep.name
                ))
            })?;
            links.insert(
                dep.name.clone(),
                ResolvedDependency {
                    kind: "workspace".into(),
                    name: dep.name,
                    spec: dep.spec,
                    group: dep.group,
                    optional: dep.optional,
                    version: None,
                    target_path: Some(target_ws.path.clone()),
                },
            );
        } else if dep.spec.starts_with("file:") {
            let relative = &dep.spec[5..];
            let target = workspace.path.join(relative).canonicalize().map_err(|_| {
                MonodepError::General(format!(
                    "File dependency '{}' target not found",
                    dep.name
                ))
            })?;
            let manifest_path = target.join("package.json");
            if !manifest_path.exists() {
                return Err(MonodepError::General(format!(
                    "File dependency '{}' points to missing package.json: {}",
                    dep.name,
                    manifest_path.display()
                )));
            }
            let manifest: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&manifest_path)?)?;
            let version = manifest
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("0.0.0")
                .to_string();
            links.insert(
                dep.name.clone(),
                ResolvedDependency {
                    kind: "file".into(),
                    name: dep.name,
                    spec: dep.spec,
                    group: dep.group,
                    optional: dep.optional,
                    version: Some(version),
                    target_path: Some(target),
                },
            );
        }
    }
    Ok(links)
}

fn sync_local_links(
    workspace: &WorkspacePackage,
    links: &BTreeMap<String, ResolvedDependency>,
) -> Result<()> {
    let node_modules = workspace.path.join("node_modules");
    let _ = remove_managed_links(&node_modules);

    if links.is_empty() {
        return Ok(());
    }

    let mut linked_deps = Vec::new();
    let mut linked_bins = Vec::new();

    for (dep_name, dep) in links {
        let target = dep.target_path.as_ref().ok_or_else(|| {
            MonodepError::General(format!("Dependency '{dep_name}' is missing a target path"))
        })?;
        relative_symlink(target, &node_modules.join(dep_name))?;
        linked_deps.push(dep_name.clone());

        let dep_manifest_path = target.join("package.json");
        if dep_manifest_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&dep_manifest_path) {
                if let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&content) {
                    for (bin_name, bin_path) in bin_entries(dep_name, &manifest) {
                        let bin_target = target.join(&bin_path);
                        if bin_target.exists() {
                            relative_symlink(&bin_target, &node_modules.join(".bin").join(&bin_name))?;
                            linked_bins.push(bin_name);
                        }
                    }
                }
            }
        }
    }

    if !linked_deps.is_empty() || !linked_bins.is_empty() {
        write_managed_marker(&node_modules, &linked_deps, &linked_bins)?;
    }
    Ok(())
}

pub fn build_plan(
    root: &Path,
    filters: &[String],
    options: &SyncOptions,
) -> Result<SyncPlan> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let workspaces = discover_workspaces(&root)?;
    let selected = closure_for_filters(&workspaces, filters, options)?;
    let pm = detect_package_manager(&root)?;

    let mut workspace_links = BTreeMap::new();
    for (ws_name, ws) in &selected {
        let links = resolve_local_deps(ws, &workspaces, options)?;
        let mut link_map = BTreeMap::new();
        for (k, v) in links {
            link_map.insert(k, v);
        }
        workspace_links.insert(ws_name.clone(), link_map);
    }

    let ws_map: BTreeMap<String, String> = workspaces
        .iter()
        .map(|(name, ws)| {
            let rel = ws
                .path
                .strip_prefix(&root)
                .unwrap_or(&ws.path)
                .to_string_lossy()
                .to_string();
            (name.clone(), rel)
        })
        .collect();

    Ok(SyncPlan {
        options: options.clone(),
        package_manager: pm,
        root,
        selected_workspaces: selected.keys().cloned().collect(),
        workspace_links,
        workspaces: ws_map,
        dedup: None,
    })
}

pub fn sync(root: &Path, filters: &[String], options: &SyncOptions) -> Result<SyncPlan> {
    let mut plan = build_plan(root, filters, options)?;
    let workspaces = discover_workspaces(&plan.root)?;

    for ws_name in &plan.selected_workspaces {
        let ws = &workspaces[ws_name];
        let local_deps = plan.workspace_links.get(ws_name).cloned().unwrap_or_default();
        let local_dep_names: HashSet<String> = local_deps.keys().cloned().collect();

        let has_pj = ws.path.join("package.json").exists();
        let has_pp = ws.path.join("pyproject.toml").exists();

        if !options.skip_install {
            if has_pj {
                run_install(&plan.root, &ws.path, &plan.package_manager, &local_dep_names)?;
            }
            if has_pp {
                run_uv_sync(&ws.path)?;
            }
        }

        if has_pj {
            sync_local_links(ws, &local_deps)?;
        }
    }

    let ws_paths: Vec<PathBuf> = plan
        .selected_workspaces
        .iter()
        .map(|name| workspaces[name].path.clone())
        .collect();
    let dedup_result = deduplicate_workspaces(&plan.root, &ws_paths)?;

    let all_ws_paths: Vec<PathBuf> = workspaces.values().map(|ws| ws.path.clone()).collect();
    prune_store(&plan.root, &all_ws_paths)?;

    plan.dedup = Some(dedup_result);
    Ok(plan)
}

pub fn doctor(root: &Path, filters: &[String], options: &SyncOptions) -> Result<(bool, serde_json::Value)> {
    let plan = build_plan(root, filters, options)?;
    let workspaces = discover_workspaces(&plan.root)?;
    let mut issues = Vec::new();

    for (ws_name, links) in &plan.workspace_links {
        let ws = &workspaces[ws_name];
        let node_modules = ws.path.join("node_modules");
        for (dep_name, dep) in links {
            let path = node_modules.join(dep_name);
            if !path.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
                issues.push(format!("Missing workspace symlink: {}", path.display()));
            } else if let Some(target) = &dep.target_path {
                if let Ok(resolved) = path.canonicalize() {
                    let canonical_target = target.canonicalize().unwrap_or_else(|_| target.clone());
                    if resolved != canonical_target {
                        issues.push(format!("Wrong symlink target: {}", path.display()));
                    }
                }
            }
        }
    }

    let payload = serde_json::json!({
        "healthy": issues.is_empty(),
        "issues": issues,
        "package_manager": plan.package_manager,
        "root": plan.root.to_string_lossy(),
    });
    Ok((issues.is_empty(), payload))
}

pub fn why(
    root: &Path,
    dependency_name: &str,
    filters: &[String],
    options: &SyncOptions,
) -> Result<serde_json::Value> {
    let plan = build_plan(root, filters, options)?;
    let workspaces = discover_workspaces(&plan.root)?;

    let mut local_reasons = Vec::new();
    let mut registry_reasons = Vec::new();

    for (ws_name, links) in &plan.workspace_links {
        if let Some(dep) = links.get(dependency_name) {
            local_reasons.push(serde_json::json!({
                "kind": dep.kind,
                "owner": ws_name,
                "dependency": dep,
            }));
        }
    }

    for ws_name in &plan.selected_workspaces {
        let ws = &workspaces[ws_name];
        for dep in dependency_entries(&ws.manifest, options) {
            if dep.name == dependency_name
                && !dep.spec.starts_with("workspace:")
                && !dep.spec.starts_with("file:")
            {
                registry_reasons.push(serde_json::json!({
                    "kind": "registry",
                    "owner": ws_name,
                    "group": dep.group,
                    "spec": dep.spec,
                }));
            }
        }
    }

    if local_reasons.is_empty() && registry_reasons.is_empty() {
        return Err(MonodepError::General(format!(
            "Dependency '{dependency_name}' was not found in the selected workspaces"
        )));
    }

    Ok(serde_json::json!({
        "dependency": dependency_name,
        "local_reasons": local_reasons,
        "registry_reasons": registry_reasons,
        "selected_workspaces": plan.selected_workspaces,
    }))
}

pub fn add_dependency(
    root: &Path,
    workspace_name: &str,
    package: &str,
    dev: bool,
    options: &SyncOptions,
) -> Result<serde_json::Value> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let workspaces = discover_workspaces(&root)?;
    let ws = workspaces.get(workspace_name).ok_or_else(|| {
        MonodepError::General(format!("Workspace '{workspace_name}' was not found"))
    })?;

    let pm = detect_package_manager(&root)?;
    let ecosystem = run_add(&ws.path, package, &pm, dev)?;
    let plan = sync(&root, &[], options)?;

    Ok(serde_json::json!({
        "dependency": package,
        "dev": dev,
        "ecosystem": ecosystem,
        "plan": plan,
        "workspace": workspace_name,
    }))
}

pub fn update_dependency(
    root: &Path,
    workspace_name: &str,
    package: Option<&str>,
    options: &SyncOptions,
) -> Result<serde_json::Value> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let workspaces = discover_workspaces(&root)?;
    let ws = workspaces.get(workspace_name).ok_or_else(|| {
        MonodepError::General(format!("Workspace '{workspace_name}' was not found"))
    })?;

    let pm = detect_package_manager(&root)?;
    let mut updated = vec![workspace_name.to_string()];
    let ecosystem = run_update(&ws.path, package, &pm)?;

    if let Some(pkg) = package {
        for (name, other_ws) in &workspaces {
            if name == workspace_name {
                continue;
            }
            if workspace_has_dependency(&other_ws.path, pkg) {
                let _ = run_update(&other_ws.path, Some(pkg), &pm);
                updated.push(name.clone());
            }
        }
    }

    updated.sort();
    let plan = sync(&root, &[], options)?;

    Ok(serde_json::json!({
        "dependency": package,
        "ecosystem": ecosystem,
        "plan": plan,
        "updated_workspaces": updated,
        "workspace": workspace_name,
    }))
}

pub fn remove_dependency(
    root: &Path,
    workspace_name: &str,
    dependency_name: &str,
    options: &SyncOptions,
) -> Result<serde_json::Value> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let workspaces = discover_workspaces(&root)?;
    let ws = workspaces.get(workspace_name).ok_or_else(|| {
        MonodepError::General(format!("Workspace '{workspace_name}' was not found"))
    })?;

    let pm = detect_package_manager(&root)?;
    let ecosystem = run_remove(&ws.path, dependency_name, &pm)?;
    let plan = sync(&root, &[], options)?;

    Ok(serde_json::json!({
        "dependency": dependency_name,
        "ecosystem": ecosystem,
        "plan": plan,
        "workspace": workspace_name,
    }))
}
