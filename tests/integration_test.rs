use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use tempfile::TempDir;

// ── Test fixture helpers ─────────────────────────────────────────────

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_string_pretty(value).unwrap() + "\n").unwrap();
}

fn create_fixture(root: &Path) {
    fs::write(
        root.join("mise.toml"),
        r#"experimental_monorepo_root = true

[monorepo]
config_roots = ["packages/*"]
package_manager = "bun"
"#,
    )
    .unwrap();

    write_json(
        &root.join("packages/shared-lib/package.json"),
        &serde_json::json!({
            "name": "shared-lib",
            "version": "1.0.0",
            "bin": { "shared-tool": "bin/shared-tool.js" }
        }),
    );
    let bin_dir = root.join("packages/shared-lib/bin");
    fs::create_dir_all(&bin_dir).unwrap();
    fs::write(
        bin_dir.join("shared-tool.js"),
        "#!/usr/bin/env node\nconsole.log('shared');\n",
    )
    .unwrap();

    write_json(
        &root.join("packages/app-a/package.json"),
        &serde_json::json!({
            "name": "app-a",
            "version": "1.0.0",
            "dependencies": {
                "shared-lib": "workspace:*",
                "left-pad": "^1.0.0",
                "local-bin": "file:../local-bin"
            },
            "devDependencies": {
                "dev-tool": "~1.2.0"
            },
            "optionalDependencies": {
                "optional-tool": "^2.0.0"
            }
        }),
    );

    write_json(
        &root.join("packages/app-b/package.json"),
        &serde_json::json!({
            "name": "app-b",
            "version": "1.0.0",
            "dependencies": { "chalk": "^1.0.0" }
        }),
    );

    write_json(
        &root.join("packages/local-bin/package.json"),
        &serde_json::json!({
            "name": "local-bin",
            "version": "1.0.0",
            "bin": "bin/local-bin.js"
        }),
    );
    let lb_bin = root.join("packages/local-bin/bin");
    fs::create_dir_all(&lb_bin).unwrap();
    fs::write(
        lb_bin.join("local-bin.js"),
        "#!/usr/bin/env node\nconsole.log('local');\n",
    )
    .unwrap();
}

fn create_bun_package(node_modules: &Path, name: &str, version: &str, files: &[(&str, &str)]) {
    let safe = name.replace('/', "+");
    let pkg_dir = node_modules
        .join(".bun")
        .join(format!("{safe}@{version}+fakehash"))
        .join("node_modules")
        .join(name);
    fs::create_dir_all(&pkg_dir).unwrap();
    write_json(
        &pkg_dir.join("package.json"),
        &serde_json::json!({"name": name, "version": version}),
    );
    for (rel, content) in files {
        let target = pkg_dir.join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&target, content).unwrap();
    }
    // symlink
    let link = node_modules.join(name);
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    if link.exists() || link.symlink_metadata().is_ok() {
        let _ = fs::remove_file(&link);
    }
    let relative = pathdiff::diff_paths(&pkg_dir, link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&relative, &link).unwrap();
}

fn create_venv_package(
    ws_path: &Path,
    name: &str,
    version: &str,
    top_level: &[&str],
    files: &[(&str, &str)],
) {
    let site = ws_path.join(".venv/lib/python3.12/site-packages");
    let safe_name = name.replace('-', "_");
    let dist = site.join(format!("{safe_name}-{version}.dist-info"));
    fs::create_dir_all(&dist).unwrap();
    fs::write(
        dist.join("METADATA"),
        format!("Name: {name}\nVersion: {version}\n\n"),
    )
    .unwrap();
    fs::write(dist.join("top_level.txt"), top_level.join("\n") + "\n").unwrap();
    for top in top_level {
        let pkg_dir = site.join(top);
        fs::create_dir_all(&pkg_dir).unwrap();
        for (rel, content) in files {
            let target = pkg_dir.join(rel);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&target, content).unwrap();
        }
    }
}

fn monodep_cmd() -> Command {
    Command::cargo_bin("monodep").unwrap()
}

// ── CLI: plan ────────────────────────────────────────────────────────

#[test]
fn cli_plan_discovers_workspaces() {
    let tmp = TempDir::new().unwrap();
    create_fixture(tmp.path());

    let output = monodep_cmd()
        .args(["plan", tmp.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap();
    let workspaces = payload["selected_workspaces"].as_array().unwrap();
    assert!(workspaces.iter().any(|v| v == "app-a"));
    assert!(workspaces.iter().any(|v| v == "app-b"));
    assert_eq!(payload["package_manager"], "bun");
}

#[test]
fn cli_plan_resolves_workspace_deps() {
    let tmp = TempDir::new().unwrap();
    create_fixture(tmp.path());

    let output = monodep_cmd()
        .args(["plan", tmp.path().to_str().unwrap()])
        .output()
        .unwrap();

    let payload: Value = serde_json::from_slice(&output.stdout).unwrap();
    let links = &payload["workspace_links"]["app-a"];
    assert_eq!(links["shared-lib"]["kind"], "workspace");
    assert_eq!(links["local-bin"]["kind"], "file");
    // Registry deps should NOT appear in workspace_links
    assert!(links.get("left-pad").is_none());
    assert!(links.get("chalk").is_none());
}

// ── CLI: sync ────────────────────────────────────────────────────────

#[test]
fn cli_sync_creates_workspace_symlinks() {
    let tmp = TempDir::new().unwrap();
    create_fixture(tmp.path());

    let output = monodep_cmd()
        .args(["sync", tmp.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success());

    let link = tmp.path().join("packages/app-a/node_modules/shared-lib");
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());

    let link2 = tmp.path().join("packages/app-a/node_modules/local-bin");
    assert!(link2.symlink_metadata().unwrap().file_type().is_symlink());
}

#[test]
fn cli_sync_creates_bin_links() {
    let tmp = TempDir::new().unwrap();
    create_fixture(tmp.path());

    monodep_cmd()
        .args(["sync", tmp.path().to_str().unwrap()])
        .assert()
        .success();

    let bin_shared = tmp
        .path()
        .join("packages/app-a/node_modules/.bin/shared-tool");
    assert!(
        bin_shared
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );

    let bin_local = tmp
        .path()
        .join("packages/app-a/node_modules/.bin/local-bin");
    assert!(
        bin_local
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn cli_sync_writes_managed_marker() {
    let tmp = TempDir::new().unwrap();
    create_fixture(tmp.path());

    monodep_cmd()
        .args(["sync", tmp.path().to_str().unwrap()])
        .assert()
        .success();

    let marker = tmp
        .path()
        .join("packages/app-a/node_modules/.monodep-managed.json");
    assert!(marker.exists());
    let data: Value = serde_json::from_str(&fs::read_to_string(&marker).unwrap()).unwrap();
    let deps = data["dependencies"].as_array().unwrap();
    assert!(deps.iter().any(|v| v == "shared-lib"));
    assert!(deps.iter().any(|v| v == "local-bin"));
}

#[test]
fn cli_sync_filtered_limits_scope() {
    let tmp = TempDir::new().unwrap();
    create_fixture(tmp.path());

    let output = monodep_cmd()
        .args(["sync", tmp.path().to_str().unwrap(), "--filter", "app-a"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap();
    let selected = payload["selected_workspaces"].as_array().unwrap();
    assert!(selected.iter().any(|v| v == "app-a"));
    assert!(selected.iter().any(|v| v == "shared-lib")); // transitive
    assert!(!selected.iter().any(|v| v == "app-b"));
}

#[test]
fn cli_sync_production_excludes_dev() {
    let tmp = TempDir::new().unwrap();
    create_fixture(tmp.path());

    let output = monodep_cmd()
        .args(["sync", tmp.path().to_str().unwrap(), "--production"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["options"]["include_dev"], false);
}

// ── CLI: doctor ──────────────────────────────────────────────────────

#[test]
fn cli_doctor_healthy_after_sync() {
    let tmp = TempDir::new().unwrap();
    create_fixture(tmp.path());

    monodep_cmd()
        .args(["sync", tmp.path().to_str().unwrap()])
        .assert()
        .success();

    let output = monodep_cmd()
        .args(["doctor", tmp.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["healthy"], true);
}

#[test]
fn cli_doctor_detects_missing_symlinks() {
    let tmp = TempDir::new().unwrap();
    create_fixture(tmp.path());

    // No sync → symlinks missing
    let output = monodep_cmd()
        .args(["doctor", tmp.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert!(!output.status.success()); // exit 1
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["healthy"], false);
}

// ── CLI: why ─────────────────────────────────────────────────────────

#[test]
fn cli_why_workspace_dep() {
    let tmp = TempDir::new().unwrap();
    create_fixture(tmp.path());

    let output = monodep_cmd()
        .args(["why", "shared-lib", tmp.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["dependency"], "shared-lib");
    let locals = payload["local_reasons"].as_array().unwrap();
    assert!(locals.iter().any(|r| r["owner"] == "app-a"));
}

#[test]
fn cli_why_registry_dep() {
    let tmp = TempDir::new().unwrap();
    create_fixture(tmp.path());

    let output = monodep_cmd()
        .args(["why", "left-pad", tmp.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap();
    let registry = payload["registry_reasons"].as_array().unwrap();
    assert!(registry.iter().any(|r| r["owner"] == "app-a"));
}

#[test]
fn cli_why_unknown_dep_fails() {
    let tmp = TempDir::new().unwrap();
    create_fixture(tmp.path());

    monodep_cmd()
        .args(["why", "nonexistent", tmp.path().to_str().unwrap()])
        .assert()
        .failure();
}

// ── Dedup: Node.js ───────────────────────────────────────────────────

fn create_dedup_fixture(root: &Path) {
    fs::write(
        root.join("mise.toml"),
        r#"experimental_monorepo_root = true

[monorepo]
config_roots = ["packages/*"]
package_manager = "bun"
"#,
    )
    .unwrap();

    write_json(
        &root.join("packages/app-a/package.json"),
        &serde_json::json!({"name": "app-a", "version": "1.0.0",
            "dependencies": {"left-pad": "^1.0.0", "zod": "^4.0.0"}}),
    );
    write_json(
        &root.join("packages/app-b/package.json"),
        &serde_json::json!({"name": "app-b", "version": "1.0.0",
            "dependencies": {"left-pad": "^1.0.0", "zod": "^4.0.0"}}),
    );

    for ws in &["app-a", "app-b"] {
        let nm = root.join(format!("packages/{ws}/node_modules"));
        create_bun_package(&nm, "left-pad", "1.3.0", &[("index.js", "pad\n")]);
        create_bun_package(
            &nm,
            "zod",
            "4.3.6",
            &[("index.js", "zod\n"), ("lib/core.js", "core\n")],
        );
    }
}

#[test]
fn dedup_creates_store_and_hardlinks() {
    let tmp = TempDir::new().unwrap();
    create_dedup_fixture(tmp.path());

    let output = monodep_cmd()
        .args(["sync", tmp.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["dedup"]["node"]["deduplicated_packages"], 2);

    // Verify hardlinks share inodes
    let a_index = tmp
        .path()
        .join("packages/app-a/node_modules/left-pad")
        .canonicalize()
        .unwrap()
        .join("index.js");
    let b_index = tmp
        .path()
        .join("packages/app-b/node_modules/left-pad")
        .canonicalize()
        .unwrap()
        .join("index.js");

    assert_eq!(
        a_index.metadata().unwrap().ino(),
        b_index.metadata().unwrap().ino(),
    );
}

#[test]
fn dedup_idempotent() {
    let tmp = TempDir::new().unwrap();
    create_dedup_fixture(tmp.path());

    // First sync
    let out1 = monodep_cmd()
        .args(["sync", tmp.path().to_str().unwrap()])
        .output()
        .unwrap();
    let p1: Value = serde_json::from_slice(&out1.stdout).unwrap();
    assert!(p1["dedup"]["node"]["files_hardlinked"].as_u64().unwrap() > 0);

    // Second sync — already hardlinked
    let out2 = monodep_cmd()
        .args(["sync", tmp.path().to_str().unwrap()])
        .output()
        .unwrap();
    let p2: Value = serde_json::from_slice(&out2.stdout).unwrap();
    assert_eq!(p2["dedup"]["node"]["files_hardlinked"], 0);
}

#[test]
fn dedup_skips_single_workspace_packages() {
    let tmp = TempDir::new().unwrap();
    create_dedup_fixture(tmp.path());

    let nm = tmp.path().join("packages/app-a/node_modules");
    create_bun_package(&nm, "unique-pkg", "1.0.0", &[("index.js", "x\n")]);

    monodep_cmd()
        .args(["sync", tmp.path().to_str().unwrap()])
        .assert()
        .success();

    assert!(!tmp.path().join(".monodep/store/node/unique-pkg").exists());
}

// ── Dedup: Python ────────────────────────────────────────────────────

#[test]
fn python_dedup_creates_store_and_hardlinks() {
    let tmp = TempDir::new().unwrap();
    create_dedup_fixture(tmp.path());

    for ws in &["app-a", "app-b"] {
        create_venv_package(
            &tmp.path().join(format!("packages/{ws}")),
            "pydantic",
            "2.13.1",
            &["pydantic"],
            &[("__init__.py", "# pydantic\n"), ("main.py", "# main\n")],
        );
    }

    let output = monodep_cmd()
        .args(["sync", tmp.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["dedup"]["python"]["deduplicated_packages"], 1);
    assert!(
        payload["dedup"]["python"]["files_hardlinked"]
            .as_u64()
            .unwrap()
            > 0
    );

    let a_init = tmp
        .path()
        .join("packages/app-a/.venv/lib/python3.12/site-packages/pydantic/__init__.py");
    let b_init = tmp
        .path()
        .join("packages/app-b/.venv/lib/python3.12/site-packages/pydantic/__init__.py");
    assert_eq!(
        a_init.metadata().unwrap().ino(),
        b_init.metadata().unwrap().ino(),
    );
}

#[test]
fn python_dedup_skips_unique() {
    let tmp = TempDir::new().unwrap();
    create_dedup_fixture(tmp.path());

    create_venv_package(
        &tmp.path().join("packages/app-a"),
        "torch",
        "2.0.0",
        &["torch"],
        &[("__init__.py", "# torch\n")],
    );

    let output = monodep_cmd()
        .args(["sync", tmp.path().to_str().unwrap()])
        .output()
        .unwrap();

    let payload: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["dedup"]["python"]["deduplicated_packages"], 0);
}
