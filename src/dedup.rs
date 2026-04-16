use crate::models::*;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const STORE_DIR: &str = ".monodep/store";
const STORE_NODE: &str = "node";
const STORE_PYTHON: &str = "python";

fn store_path(root: &Path, ecosystem: &str, key: &str) -> PathBuf {
    let (name, version) = key.rsplit_once('@').unwrap_or((key, "0.0.0"));
    let safe_name = name.replace('/', "+");
    root.join(STORE_DIR).join(ecosystem).join(safe_name).join(version)
}

// ── Node.js scanning ────────────────────────────────────────────────

fn scan_bun_layout(node_modules: &Path) -> BTreeMap<String, Vec<PathBuf>> {
    let mut results: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let bun_dir = node_modules.join(".bun");
    if !bun_dir.exists() {
        return results;
    }
    let Ok(entries) = fs::read_dir(&bun_dir) else {
        return results;
    };
    for entry in entries.flatten() {
        if !entry.path().is_dir() || entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let nm = entry.path().join("node_modules");
        if !nm.exists() {
            continue;
        }
        let Ok(pkg_dirs) = fs::read_dir(&nm) else {
            continue;
        };
        for pkg_entry in pkg_dirs.flatten() {
            let pkg_path = pkg_entry.path();
            if pkg_path.is_symlink()
                || !pkg_path.is_dir()
                || pkg_path.file_name().unwrap_or_default().to_string_lossy().starts_with('.')
            {
                continue;
            }
            if pkg_path.file_name().unwrap_or_default().to_string_lossy().starts_with('@') {
                if let Ok(scoped) = fs::read_dir(&pkg_path) {
                    for s in scoped.flatten() {
                        if !s.path().is_symlink() && s.path().is_dir() {
                            try_add_node_package(&s.path(), &mut results);
                        }
                    }
                }
            } else {
                try_add_node_package(&pkg_path, &mut results);
            }
        }
    }
    results
}

fn scan_flat_layout(node_modules: &Path) -> BTreeMap<String, Vec<PathBuf>> {
    let mut results: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let Ok(entries) = fs::read_dir(node_modules) else {
        return results;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_symlink()
            || !path.is_dir()
            || path.file_name().unwrap_or_default().to_string_lossy().starts_with('.')
        {
            continue;
        }
        if path.file_name().unwrap_or_default().to_string_lossy().starts_with('@') {
            if let Ok(scoped) = fs::read_dir(&path) {
                for s in scoped.flatten() {
                    if !s.path().is_symlink() && s.path().is_dir() {
                        try_add_node_package(&s.path(), &mut results);
                    }
                }
            }
        } else {
            try_add_node_package(&path, &mut results);
        }
    }
    results
}

fn try_add_node_package(pkg_dir: &Path, results: &mut BTreeMap<String, Vec<PathBuf>>) {
    let pj = pkg_dir.join("package.json");
    if !pj.exists() {
        return;
    }
    let Ok(content) = fs::read_to_string(&pj) else {
        return;
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&content) else {
        return;
    };
    let name = manifest.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let version = manifest.get("version").and_then(|v| v.as_str()).unwrap_or("");
    if name.is_empty() || version.is_empty() {
        return;
    }
    let key = format!("{name}@{version}");
    results.entry(key).or_default().push(pkg_dir.to_path_buf());
}

pub fn scan_node_packages(node_modules: &Path) -> BTreeMap<String, Vec<PathBuf>> {
    if !node_modules.exists() {
        return BTreeMap::new();
    }
    if node_modules.join(".bun").exists() {
        scan_bun_layout(node_modules)
    } else {
        scan_flat_layout(node_modules)
    }
}

// ── Python scanning ─────────────────────────────────────────────────

fn find_site_packages(venv: &Path) -> Option<PathBuf> {
    let lib = venv.join("lib");
    if !lib.exists() {
        return None;
    }
    if let Ok(entries) = fs::read_dir(&lib) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("python") {
                let site = entry.path().join("site-packages");
                if site.exists() {
                    return Some(site);
                }
            }
        }
    }
    None
}

fn parse_dist_info_key(dist_info: &Path) -> Option<String> {
    let metadata = dist_info.join("METADATA");
    if !metadata.exists() {
        return None;
    }
    let content = fs::read_to_string(&metadata).ok()?;
    let mut name = None;
    let mut version = None;
    for line in content.lines() {
        if line.is_empty() {
            break;
        }
        if let Some(n) = line.strip_prefix("Name:") {
            name = Some(n.trim().to_string());
        } else if let Some(v) = line.strip_prefix("Version:") {
            version = Some(v.trim().to_string());
        }
        if name.is_some() && version.is_some() {
            break;
        }
    }
    Some(format!("{}@{}", name?, version?))
}

fn dist_info_top_level(dist_info: &Path) -> Vec<String> {
    let top_level = dist_info.join("top_level.txt");
    if top_level.exists() {
        if let Ok(content) = fs::read_to_string(&top_level) {
            return content.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        }
    }
    Vec::new()
}

pub fn scan_python_packages(venv: &Path) -> BTreeMap<String, Vec<PathBuf>> {
    let mut results: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let Some(site) = find_site_packages(venv) else {
        return results;
    };
    let Ok(entries) = fs::read_dir(&site) else {
        return results;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || !path.to_string_lossy().ends_with(".dist-info") {
            continue;
        }
        let Some(key) = parse_dist_info_key(&path) else {
            continue;
        };
        let mut dirs = vec![path.clone()];
        for top_name in dist_info_top_level(&path) {
            let pkg_dir = site.join(&top_name);
            if pkg_dir.is_dir() && !pkg_dir.is_symlink() {
                dirs.push(pkg_dir);
            }
        }
        results.entry(key).or_default().extend(dirs);
    }
    results
}

// ── Hardlink engine ─────────────────────────────────────────────────

fn hardlink_dir(source: &Path, target: &Path) -> Result<usize> {
    let mut count = 0;
    for entry in walkdir::WalkDir::new(source).into_iter().flatten() {
        if !entry.file_type().is_file() || entry.path_is_symlink() {
            continue;
        }
        let relative = entry.path().strip_prefix(source).unwrap();
        let tgt_file = target.join(relative);
        if !tgt_file.exists() || tgt_file.symlink_metadata()?.file_type().is_symlink() {
            continue;
        }
        let src_ino = entry.metadata()?.ino();
        let tgt_ino = tgt_file.metadata()?.ino();
        if src_ino == tgt_ino {
            continue;
        }
        fs::remove_file(&tgt_file)?;
        fs::hard_link(entry.path(), &tgt_file)?;
        count += 1;
    }
    Ok(count)
}

fn deduplicate_index(
    root: &Path,
    ecosystem: &str,
    global_index: &BTreeMap<String, Vec<Vec<PathBuf>>>,
) -> Result<DedupStats> {
    let mut stats = DedupStats {
        packages_scanned: global_index.len(),
        ..Default::default()
    };

    for (key, ws_dir_lists) in global_index {
        if ws_dir_lists.len() < 2 {
            continue;
        }

        let first_dirs = &ws_dir_lists[0];
        for pkg_dir in first_dirs {
            let dir_name = pkg_dir.file_name().unwrap_or_default().to_string_lossy();
            let sp = store_path(root, ecosystem, key).join(dir_name.as_ref());
            if !sp.exists() {
                if let Some(parent) = sp.parent() {
                    fs::create_dir_all(parent)?;
                }
                copy_dir_all(pkg_dir, &sp)?;
            }
        }

        for dir_list in ws_dir_lists {
            for pkg_dir in dir_list {
                let dir_name = pkg_dir.file_name().unwrap_or_default().to_string_lossy();
                let sp = store_path(root, ecosystem, key).join(dir_name.as_ref());
                if sp.exists() {
                    stats.files_hardlinked += hardlink_dir(&sp, pkg_dir)?;
                }
            }
        }

        stats.deduplicated_packages += 1;
        stats.duplicate_copies_saved += ws_dir_lists.len() - 1;
    }

    Ok(stats)
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in walkdir::WalkDir::new(src).into_iter().flatten() {
        let relative = entry.path().strip_prefix(src).unwrap();
        let target = dst.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

// ── Public API ──────────────────────────────────────────────────────

pub fn deduplicate_workspaces(root: &Path, workspace_paths: &[PathBuf]) -> Result<DedupResult> {
    // Node.js
    let mut node_index: BTreeMap<String, Vec<Vec<PathBuf>>> = BTreeMap::new();
    for ws_path in workspace_paths {
        let packages = scan_node_packages(&ws_path.join("node_modules"));
        for (key, dirs) in packages {
            node_index.entry(key).or_default().push(dirs);
        }
    }

    // Python
    let mut python_index: BTreeMap<String, Vec<Vec<PathBuf>>> = BTreeMap::new();
    for ws_path in workspace_paths {
        let packages = scan_python_packages(&ws_path.join(".venv"));
        for (key, dirs) in packages {
            python_index.entry(key).or_default().push(dirs);
        }
    }

    let node_stats = deduplicate_index(root, STORE_NODE, &node_index)?;
    let python_stats = deduplicate_index(root, STORE_PYTHON, &python_index)?;

    let store_root = root.join(STORE_DIR);
    let has_store = store_root.join(STORE_NODE).exists() || store_root.join(STORE_PYTHON).exists();

    Ok(DedupResult {
        node: node_stats,
        python: python_stats,
        store: if has_store {
            Some(store_root.to_string_lossy().to_string())
        } else {
            None
        },
    })
}

pub fn prune_store(root: &Path, workspace_paths: &[PathBuf]) -> Result<usize> {
    let store_root = root.join(STORE_DIR);
    if !store_root.exists() {
        return Ok(0);
    }

    let mut active_node: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut active_python: std::collections::HashSet<String> = std::collections::HashSet::new();
    for ws in workspace_paths {
        for key in scan_node_packages(&ws.join("node_modules")).keys() {
            active_node.insert(key.clone());
        }
        for key in scan_python_packages(&ws.join(".venv")).keys() {
            active_python.insert(key.clone());
        }
    }

    let mut removed = 0;
    for (ecosystem, active) in [
        (STORE_NODE, &active_node),
        (STORE_PYTHON, &active_python),
    ] {
        let eco_dir = store_root.join(ecosystem);
        if !eco_dir.exists() {
            continue;
        }
        let Ok(name_entries) = fs::read_dir(&eco_dir) else {
            continue;
        };
        for name_entry in name_entries.flatten() {
            if !name_entry.path().is_dir() {
                continue;
            }
            let Ok(version_entries) = fs::read_dir(name_entry.path()) else {
                continue;
            };
            for version_entry in version_entries.flatten() {
                if !version_entry.path().is_dir() {
                    continue;
                }
                let name = name_entry
                    .file_name()
                    .to_string_lossy()
                    .replace('+', "/");
                let version = version_entry.file_name().to_string_lossy().to_string();
                let key = format!("{name}@{version}");
                if !active.contains(&key) {
                    fs::remove_dir_all(version_entry.path())?;
                    removed += 1;
                }
            }
            // Remove empty name dir
            if name_entry.path().exists()
                && fs::read_dir(name_entry.path())
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(false)
            {
                fs::remove_dir(name_entry.path())?;
            }
        }
    }

    Ok(removed)
}
