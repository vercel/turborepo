//! CLI contracts for generic native repository bootstrap, not native execution.
//! Plain `ls` should need only in-process package inventories. Every invocation
//! uses an empty PATH, so a regression requiring Cargo, uv, Go, or a JavaScript
//! package manager fails instead of being hidden by the host's installed tools.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path, process::Output};

use serde_json::{Value, json};

#[derive(Clone, Copy)]
struct Workspace {
    fixture: &'static str,
    flag: &'static str,
    marker: &'static str,
    invalid_marker: &'static str,
    member: &'static str,
    packages: &'static [(&'static str, &'static str)],
}

const CARGO: Workspace = Workspace {
    fixture: "cargo_pure_workspace",
    flag: "experimentalCargoWorkspaces",
    marker: "Cargo.toml",
    invalid_marker: "[workspace\n",
    member: "crates/app",
    packages: &[
        ("acme", ""),
        ("app", "crates/app"),
        ("lib-a", "crates/lib-a"),
    ],
};
const UV: Workspace = Workspace {
    fixture: "uv_pure_workspace",
    flag: "experimentalPythonWorkspaces",
    marker: "pyproject.toml",
    invalid_marker: "[tool.uv.workspace\n",
    member: "packages/py-app",
    packages: &[
        ("acme", ""),
        ("py-app", "packages/py-app"),
        ("py-lib", "packages/py-lib"),
    ],
};
const GO: Workspace = Workspace {
    fixture: "go_pure_workspace",
    flag: "experimentalGoWorkspaces",
    marker: "go.work",
    invalid_marker: "go 1.22\nuse (\n",
    member: "apps/api",
    packages: &[
        ("api", "apps/api"),
        ("go-workspace", ""),
        ("lib", "packages/lib"),
    ],
};

fn fixture(workspace: Workspace) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    // Copy only; do not install packages or initialize/commit a Git repository.
    // These fixtures contain local-only manifests and any required lock data.
    common::setup::copy_fixture(workspace.fixture, dir.path()).unwrap();
    // Bound inference to this fixture, without invoking Git or creating commits.
    fs::create_dir(dir.path().join(".git")).unwrap();
    assert!(!dir.path().join("package.json").exists());
    write_config(&dir.path().join("turbo.json"), workspace, true);
    dir
}

fn write_config(path: &Path, workspace: Workspace, enabled: bool) {
    fs::write(
        path,
        serde_json::to_vec_pretty(&json!({
            "futureFlags": { (workspace.flag): enabled },
            "tasks": {}
        }))
        .unwrap(),
    )
    .unwrap();
}

fn unrelated_package_json(dir: &Path) {
    // Deliberately no packageManager/workspaces: native bootstrap must not need
    // irrelevant JavaScript metadata to supply a package manager or root mode.
    fs::write(
        dir.join("package.json"),
        r#"{"name":"unrelated-js-package","private":true}"#,
    )
    .unwrap();
}

fn ls(cwd: &Path, config: Option<&Path>) -> Output {
    let environment = tempfile::tempdir().unwrap();
    let empty_path = environment.path().join("empty-path");
    let home = environment.path().join("home");
    fs::create_dir(&empty_path).unwrap();
    fs::create_dir(&home).unwrap();
    let mut command = common::turbo_command(cwd);
    command
        .env("PATH", &empty_path)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", &home)
        .env(
            "TURBO_CONFIG_DIR_PATH",
            environment.path().join("turbo-config"),
        )
        .env("TURBO_DOWNLOAD_LOCAL_ENABLED", "0")
        .env("GOENV", "off")
        .env("GOTOOLCHAIN", "local")
        .env_remove("GOWORK")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_HOME")
        .args(["ls", "--output=json"]);
    if let Some(config) = config {
        command.arg("--root-turbo-json").arg(config);
    }
    command.output().expect("execute the actual turbo binary")
}

fn assert_inventory(output: &Output, workspace: Workspace) -> Value {
    let diagnostics = common::combined_output(output);
    assert!(output.status.success(), "{}\n{diagnostics}", output.status);
    let result: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("ls must emit JSON: {error}\n{diagnostics}"));
    assert_eq!(result["packageManager"], "", "{result}");
    let packages = result["packages"]["items"].as_array().unwrap();
    let mut actual: Vec<_> = packages
        .iter()
        .map(|package| {
            (
                package["name"].as_str().unwrap().to_owned(),
                package["path"].as_str().unwrap().replace('\\', "/"),
            )
        })
        .collect();
    actual.sort();
    let mut expected: Vec<_> = workspace
        .packages
        .iter()
        .map(|(name, path)| (name.to_string(), path.to_string()))
        .collect();
    expected.sort();
    assert_eq!(actual, expected, "{diagnostics}");
    assert_eq!(result["packages"]["count"], expected.len(), "{result}");
    result
}

fn no_root_package_json(workspace: Workspace) {
    let dir = fixture(workspace);
    assert_inventory(&ls(dir.path(), None), workspace);
}

fn irrelevant_root_package_json(workspace: Workspace) {
    let dir = fixture(workspace);
    unrelated_package_json(dir.path());
    assert_inventory(&ls(dir.path(), None), workspace);
}

fn nested_member_package_json(workspace: Workspace) {
    let dir = fixture(workspace);
    let member = dir.path().join(workspace.member);
    unrelated_package_json(&member);
    // A package-level config must not steal the root's native opt-in either.
    fs::write(
        member.join("turbo.json"),
        r#"{"extends":["//"],"tasks":{}}"#,
    )
    .unwrap();
    let nested = assert_inventory(&ls(&member, None), workspace);
    let root = assert_inventory(&ls(dir.path(), None), workspace);
    assert_eq!(
        nested, root,
        "entrypoint must not change the repository inventory"
    );
}

fn explicit_config(workspace: Workspace, absolute: bool) {
    let dir = fixture(workspace);
    let custom = dir.path().join("selected.json");
    write_config(&custom, workspace, true);
    // Only the selected, nonstandard filename enables native discovery.
    write_config(&dir.path().join("turbo.json"), workspace, false);
    unrelated_package_json(&dir.path().join(workspace.member));
    for relative_cwd in ["", workspace.member] {
        let cwd = dir.path().join(relative_cwd);
        let selected = if absolute {
            custom.clone()
        } else if relative_cwd.is_empty() {
            "selected.json".into()
        } else {
            "../../selected.json".into()
        };
        assert_inventory(&ls(&cwd, Some(&selected)), workspace);
    }
}

fn selected_config_does_not_borrow_default_flags(workspace: Workspace) {
    let dir = fixture(workspace);
    let custom = dir.path().join("selected.json");
    write_config(&custom, workspace, false);
    unrelated_package_json(&dir.path().join(workspace.member));
    for relative_cwd in ["", workspace.member] {
        let cwd = dir.path().join(relative_cwd);
        let relative_config = if relative_cwd.is_empty() {
            Path::new("selected.json")
        } else {
            Path::new("../../selected.json")
        };
        for selected in [custom.as_path(), relative_config] {
            let output = ls(&cwd, Some(selected));
            let diagnostics = common::combined_output(&output);
            assert!(!output.status.success(), "{diagnostics}");
            assert!(
                diagnostics.contains("Failed to find repository root")
                    || diagnostics.contains("Unable to read package.json")
                    || (diagnostics.contains("Could not resolve workspace")
                        && diagnostics.contains("packageManager")),
                "disabled selected config must not borrow turbo.json flags or require a native \
                 subprocess:\n{diagnostics}"
            );
        }
    }
}

fn invalid_enabled_marker_is_not_ignored(workspace: Workspace) {
    let dir = fixture(workspace);
    unrelated_package_json(&dir.path().join(workspace.member));
    fs::write(dir.path().join(workspace.marker), workspace.invalid_marker).unwrap();
    // Also cover the tempting fallback to a valid, irrelevant JS root.
    for root_package_json in [false, true] {
        if root_package_json {
            unrelated_package_json(dir.path());
        }
        for relative_cwd in ["", workspace.member] {
            let output = ls(&dir.path().join(relative_cwd), None);
            let diagnostics = common::combined_output(&output);
            assert!(!output.status.success(), "{diagnostics}");
            assert!(
                diagnostics.contains(workspace.marker) && diagnostics.contains("workspace at"),
                "must retain native bootstrap diagnostic, not silently drop native \
                 support:\n{diagnostics}"
            );
            assert!(
                !diagnostics.contains("Failed to find repository root")
                    && !diagnostics.contains("Missing `packageManager`"),
                "native error must not become a generic/JavaScript fallback:\n{diagnostics}"
            );
        }
    }
}

macro_rules! bootstrap_contracts {
    ($module:ident, $workspace:ident) => {
        mod $module {
            use super::*;

            #[test]
            fn no_root_package_json() {
                super::no_root_package_json($workspace);
            }

            #[test]
            fn irrelevant_root_package_json() {
                super::irrelevant_root_package_json($workspace);
            }

            #[test]
            fn nested_member_package_json() {
                super::nested_member_package_json($workspace);
            }

            #[test]
            fn absolute_selected_config() {
                explicit_config($workspace, true);
            }

            #[test]
            fn invocation_relative_selected_config() {
                explicit_config($workspace, false);
            }

            #[test]
            fn selected_config_does_not_borrow_default_flags() {
                super::selected_config_does_not_borrow_default_flags($workspace);
            }

            #[test]
            fn invalid_enabled_marker_is_not_ignored() {
                super::invalid_enabled_marker_is_not_ignored($workspace);
            }
        }
    };
}

bootstrap_contracts!(cargo, CARGO);
bootstrap_contracts!(uv, UV);
bootstrap_contracts!(go, GO);

#[test]
fn displaced_native_config_selects_flags_not_root() {
    for workspace in [CARGO, UV, GO] {
        let dir = fixture(workspace);
        let external = tempfile::tempdir().unwrap();
        let configs = dir.path().join("configs");
        fs::create_dir(&configs).unwrap();
        // The selected file is authoritative even when defaults cannot be read.
        fs::write(dir.path().join("turbo.json"), "{").unwrap();
        unrelated_package_json(&dir.path().join(workspace.member));
        for custom in [
            configs.join("selected.json"),
            external.path().join("selected.json"),
        ] {
            write_config(&custom, workspace, true);
            for cwd in [dir.path().to_path_buf(), dir.path().join(workspace.member)] {
                assert_inventory(&ls(&cwd, Some(&custom)), workspace);
            }
        }
        assert_inventory(
            &ls(dir.path(), Some(Path::new("configs/selected.json"))),
            workspace,
        );
        assert_inventory(
            &ls(
                &dir.path().join(workspace.member),
                Some(Path::new("../../configs/selected.json")),
            ),
            workspace,
        );
    }
}

#[test]
fn displaced_javascript_config_does_not_reanchor_root() {
    let dir = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("configs")).unwrap();
    fs::write(
        dir.path().join("package.json"),
        r#"{"name":"root","packageManager":"npm@10.0.0"}"#,
    )
    .unwrap();
    for custom in [
        dir.path().join("configs/selected.json"),
        external.path().join("selected.json"),
    ] {
        fs::write(&custom, r#"{"tasks":{}}"#).unwrap();
        let output = ls(dir.path(), Some(&custom));
        assert!(
            output.status.success(),
            "{}",
            common::combined_output(&output)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["packageManager"], "npm");
    }
    let output = ls(dir.path(), Some(Path::new("configs/selected.json")));
    assert!(
        output.status.success(),
        "{}",
        common::combined_output(&output)
    );
}

#[test]
fn malformed_default_ancestor_requires_javascript_ownership() {
    for owns in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        let mut package = json!({"name":"outer", "packageManager":"npm@10.0.0"});
        if owns {
            package["workspaces"] = json!(["child"]);
        }
        fs::write(
            dir.path().join("package.json"),
            serde_json::to_vec(&package).unwrap(),
        )
        .unwrap();
        fs::write(dir.path().join("turbo.json"), "{").unwrap();
        let child = dir.path().join("child");
        fs::create_dir(&child).unwrap();
        fs::write(
            child.join("package.json"),
            r#"{"name":"child","packageManager":"npm@10.0.0"}"#,
        )
        .unwrap();
        fs::write(child.join("turbo.json"), r#"{"tasks":{}}"#).unwrap();
        let output = ls(&child, None);
        let diagnostics = common::combined_output(&output);
        if owns {
            assert!(!output.status.success(), "{diagnostics}");
            assert!(
                diagnostics.contains("Unable to read repository configuration"),
                "{diagnostics}"
            );
        } else {
            assert!(output.status.success(), "{diagnostics}");
            let result: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(result["packageManager"], "npm");
        }
    }
}
