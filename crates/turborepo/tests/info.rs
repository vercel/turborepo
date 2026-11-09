#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::fs;

use serde_json::{Value, json};

const PNPM_WORKSPACE: &str = "packages: []\n";
const PNPM_LOCK: &str = "lockfileVersion: '9.0'

settings:
  autoInstallPeers: true
  excludeLinksFromLockfile: false

importers:

  .: {}
";

struct PnpmRepo {
    package_json: Value,
    pnpm_lock: bool,
}

fn info_package_manager(repo: PnpmRepo) -> String {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut package_json = json!({"name": "info-fixture", "private": true});
    package_json
        .as_object_mut()
        .unwrap()
        .extend(repo.package_json.as_object().unwrap().clone());
    fs::write(root.join("package.json"), package_json.to_string()).unwrap();
    fs::write(root.join("pnpm-workspace.yaml"), PNPM_WORKSPACE).unwrap();
    fs::write(
        root.join("turbo.json"),
        json!({"agentGuidance": false, "tasks": {}}).to_string(),
    )
    .unwrap();
    if repo.pnpm_lock {
        fs::write(root.join("pnpm-lock.yaml"), PNPM_LOCK).unwrap();
    }

    let config_dir = tempfile::tempdir().unwrap();
    let output = common::turbo_command(root)
        .env("TURBO_CONFIG_DIR_PATH", config_dir.path())
        .args(["info", "--skip-infer"])
        .timeout(std::time::Duration::from_secs(30))
        .output()
        .unwrap();
    let combined = common::combined_output(&output);
    assert!(output.status.success(), "turbo info failed: {combined}");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().strip_prefix("Package manager: "))
        .unwrap_or_else(|| panic!("no `Package manager:` line in: {combined}"))
        .to_owned()
}

#[test]
fn reports_declared_package_manager_field() {
    let repo = PnpmRepo {
        package_json: json!({"packageManager": "pnpm@12.0.0"}),
        pnpm_lock: false,
    };
    assert_eq!(info_package_manager(repo), "pnpm@12.0.0");
}

#[test]
fn reports_declared_dev_engines_range() {
    let repo = PnpmRepo {
        package_json: json!({
            "devEngines": {"packageManager": {"name": "pnpm", "version": "^12.0.0"}}
        }),
        pnpm_lock: false,
    };
    assert_eq!(info_package_manager(repo), "pnpm@^12.0.0");
}

#[test]
fn package_manager_field_takes_precedence_over_dev_engines() {
    let repo = PnpmRepo {
        package_json: json!({
            "packageManager": "pnpm@12.0.0",
            "devEngines": {"packageManager": {"name": "pnpm", "version": "^12.0.0"}}
        }),
        pnpm_lock: false,
    };
    assert_eq!(info_package_manager(repo), "pnpm@12.0.0");
}

#[test]
fn url_declaration_reports_canonical_family() {
    let repo = PnpmRepo {
        package_json: json!({
            "packageManager": "pnpm@https://registry.npmjs.org/pnpm/-/pnpm-12.0.0.tgz"
        }),
        pnpm_lock: true,
    };
    assert_eq!(info_package_manager(repo), "pnpm");
}

#[test]
fn lockfile_without_declaration_reports_not_found() {
    let repo = PnpmRepo {
        package_json: json!({}),
        pnpm_lock: true,
    };
    assert_eq!(info_package_manager(repo), "Not found");
}
