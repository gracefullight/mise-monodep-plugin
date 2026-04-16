use crate::models::*;
use std::path::Path;
use std::process::Command;

const LOCKFILE_MAP: &[(&str, &str)] = &[
    ("bun", "bun.lock"),
    ("pnpm", "pnpm-lock.yaml"),
    ("npm", "package-lock.json"),
    ("yarn", "yarn.lock"),
];

pub fn detect_package_manager(root: &Path) -> Result<String> {
    // 1. Check mise.toml [monorepo].package_manager
    let config_path = root.join("mise.toml");
    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        let parsed: toml::Value = toml::from_str(&content)?;
        if let Some(pm) = parsed
            .get("monorepo")
            .and_then(|m| m.get("package_manager"))
            .and_then(|p| p.as_str())
        {
            return Ok(pm.to_string());
        }
    }

    // 2. Check lockfiles
    for &(pm, lockfile) in LOCKFILE_MAP {
        if root.join(lockfile).exists() {
            return Ok(pm.to_string());
        }
    }

    // 3. Check PATH
    for pm in &["bun", "pnpm", "npm"] {
        if which::which(pm).is_ok() {
            return Ok(pm.to_string());
        }
    }

    Err(MonodepError::General(
        "No package manager detected. Set [monorepo].package_manager in mise.toml".into(),
    ))
}

pub fn prepare_manifest_for_install(
    manifest: &serde_json::Value,
    local_dep_names: &std::collections::HashSet<String>,
) -> serde_json::Value {
    let mut result = manifest.clone();
    for group in &["dependencies", "devDependencies", "optionalDependencies"] {
        if let Some(deps) = result.get_mut(group).and_then(|d| d.as_object_mut()) {
            deps.retain(|k, _| !local_dep_names.contains(k));
            if deps.is_empty() {
                result.as_object_mut().unwrap().remove(*group);
            }
        }
    }
    result
}

fn run_cmd(cmd: &[&str], cwd: &Path) -> Result<()> {
    let output = Command::new(cmd[0])
        .args(&cmd[1..])
        .current_dir(cwd)
        .output()
        .map_err(|e| MonodepError::General(format!("'{}' not installed: {e}", cmd[0])))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(MonodepError::General(format!(
            "'{cmd}' failed: {stderr}",
            cmd = cmd.join(" ")
        )));
    }
    Ok(())
}

pub fn run_install(
    _root: &Path,
    workspace_path: &Path,
    pm: &str,
    local_dep_names: &std::collections::HashSet<String>,
) -> Result<()> {
    let manifest_path = workspace_path.join("package.json");
    let original = std::fs::read_to_string(&manifest_path)?;

    if !local_dep_names.is_empty() {
        let manifest: serde_json::Value = serde_json::from_str(&original)?;
        let cleaned = prepare_manifest_for_install(&manifest, local_dep_names);
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&cleaned)? + "\n",
        )?;
    }

    let install_cmd: Vec<&str> = match pm {
        "bun" => vec!["bun", "install"],
        "pnpm" => vec!["pnpm", "install"],
        "npm" => vec!["npm", "install"],
        "yarn" => vec!["yarn", "install"],
        _ => return Err(MonodepError::General(format!("Unsupported PM: {pm}"))),
    };

    let result = run_cmd(&install_cmd, workspace_path);

    if !local_dep_names.is_empty() {
        std::fs::write(&manifest_path, &original)?;
    }

    result
}

pub fn run_uv_sync(workspace_path: &Path) -> Result<()> {
    run_cmd(&["uv", "sync"], workspace_path)
}

pub fn run_add(workspace_path: &Path, package: &str, pm: &str, dev: bool) -> Result<String> {
    let has_pj = workspace_path.join("package.json").exists();
    let has_pp = workspace_path.join("pyproject.toml").exists();

    if has_pj {
        let mut cmd: Vec<&str> = match pm {
            "bun" => vec!["bun", "add"],
            "pnpm" => vec!["pnpm", "add"],
            "npm" => vec!["npm", "install"],
            "yarn" => vec!["yarn", "add"],
            _ => return Err(MonodepError::General(format!("Unsupported PM: {pm}"))),
        };
        if dev {
            cmd.push(if pm == "bun" { "--dev" } else { "--save-dev" });
        }
        cmd.push(package);
        run_cmd(&cmd, workspace_path)?;
        return Ok("node".into());
    }

    if has_pp {
        let mut cmd = vec!["uv", "add"];
        if dev {
            cmd.push("--dev");
        }
        cmd.push(package);
        run_cmd(&cmd, workspace_path)?;
        return Ok("python".into());
    }

    Err(MonodepError::General(format!(
        "Workspace '{}' has no package.json or pyproject.toml",
        workspace_path.display()
    )))
}

pub fn run_remove(workspace_path: &Path, package: &str, pm: &str) -> Result<String> {
    let has_pj = workspace_path.join("package.json").exists();
    let has_pp = workspace_path.join("pyproject.toml").exists();

    if has_pj {
        let cmd: Vec<&str> = match pm {
            "bun" => vec!["bun", "remove", package],
            "pnpm" => vec!["pnpm", "remove", package],
            "npm" => vec!["npm", "uninstall", package],
            "yarn" => vec!["yarn", "remove", package],
            _ => return Err(MonodepError::General(format!("Unsupported PM: {pm}"))),
        };
        run_cmd(&cmd, workspace_path)?;
        return Ok("node".into());
    }

    if has_pp {
        run_cmd(&["uv", "remove", package], workspace_path)?;
        return Ok("python".into());
    }

    Err(MonodepError::General(format!(
        "Workspace '{}' has no package.json or pyproject.toml",
        workspace_path.display()
    )))
}

pub fn run_update(workspace_path: &Path, package: Option<&str>, pm: &str) -> Result<String> {
    let has_pj = workspace_path.join("package.json").exists();
    let has_pp = workspace_path.join("pyproject.toml").exists();

    if has_pj {
        let mut cmd: Vec<&str> = match pm {
            "bun" => vec!["bun", "update"],
            "pnpm" => vec!["pnpm", "update"],
            "npm" => vec!["npm", "update"],
            "yarn" => vec!["yarn", "up"],
            _ => return Err(MonodepError::General(format!("Unsupported PM: {pm}"))),
        };
        if let Some(pkg) = package {
            cmd.push(pkg);
        }
        run_cmd(&cmd, workspace_path)?;
        return Ok("node".into());
    }

    if has_pp {
        if let Some(pkg) = package {
            run_cmd(&["uv", "lock", "--upgrade-package", pkg], workspace_path)?;
        } else {
            run_cmd(&["uv", "lock", "--upgrade"], workspace_path)?;
        }
        run_cmd(&["uv", "sync"], workspace_path)?;
        return Ok("python".into());
    }

    Err(MonodepError::General(format!(
        "Workspace '{}' has no package.json or pyproject.toml",
        workspace_path.display()
    )))
}

pub fn workspace_has_dependency(workspace_path: &Path, package: &str) -> bool {
    if let Ok(content) = std::fs::read_to_string(workspace_path.join("package.json"))
        && let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&content)
    {
        for group in &["dependencies", "devDependencies", "optionalDependencies"] {
            if manifest
                .get(group)
                .and_then(|d| d.as_object())
                .is_some_and(|deps| deps.contains_key(package))
            {
                return true;
            }
        }
    }
    if let Ok(content) = std::fs::read_to_string(workspace_path.join("pyproject.toml")) {
        return content.contains(package);
    }
    false
}
