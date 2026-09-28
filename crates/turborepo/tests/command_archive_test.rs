#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path, process::Output};

use common::{run_turbo, setup, turbo_output_filters};
use serde_json::{Value, json};
use tempfile::TempDir;

fn basic_monorepo() -> TempDir {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();
    tempdir
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "turbo failed\nstdout:\n{}\nstderr:\n{}",
        stdout(output),
        stderr(output)
    );
}

fn assert_failure(output: &Output) {
    assert!(
        !output.status.success(),
        "turbo unexpectedly succeeded\nstdout:\n{}",
        stdout(output)
    );
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn package_paths(dir: &Path) -> Vec<String> {
    let output = run_turbo(dir, &["ls", "--output", "json"]);
    assert_success(&output);
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    json["packages"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["path"].as_str().unwrap().to_owned())
        .collect()
}

fn git_exclude_file(dir: &Path) -> std::path::PathBuf {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--git-path", "info/exclude"])
        .current_dir(dir)
        .output()
        .unwrap();
    dir.join(String::from_utf8(output.stdout).unwrap().trim())
}

fn set_root_workspaces(dir: &Path, workspaces: Value) {
    let path = dir.join("package.json");
    let mut package_json = read_json(&path);
    package_json["workspaces"] = workspaces;
    fs::write(
        path,
        serde_json::to_string_pretty(&package_json).unwrap() + "\n",
    )
    .unwrap();
}

#[test]
fn archive_moves_package_out_of_the_workspace() {
    let repo = basic_monorepo();
    let dir = repo.path();
    let root_package_json = fs::read_to_string(dir.join("package.json")).unwrap();

    let output = run_turbo(dir, &["archive", "another"]);
    assert_success(&output);

    assert!(!dir.join("packages/another").exists());
    assert!(dir.join("_archived/another/package.json").is_file());
    assert_eq!(
        read_json(&dir.join("_archived/another/.turbo-archive.json")),
        json!({
            "name": "another",
            "originalPath": "packages/another",
            "workspaceEntry": null,
            "gitExclude": []
        })
    );
    assert_eq!(package_paths(dir), ["apps/my-app", "packages/util"]);
    assert_eq!(
        fs::read_to_string(dir.join("package.json")).unwrap(),
        root_package_json,
        "glob workspace entries stay untouched"
    );
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("archive_stdout", stdout(&output));
    });
}

#[test]
fn unarchive_restores_the_package() {
    let repo = basic_monorepo();
    let dir = repo.path();
    assert_success(&run_turbo(dir, &["archive", "another"]));

    let output = run_turbo(dir, &["unarchive", "another"]);
    assert_success(&output);

    assert_eq!(
        package_paths(dir),
        ["packages/another", "apps/my-app", "packages/util"]
    );
    assert!(!dir.join("_archived").exists());
    assert!(!dir.join("packages/another/.turbo-archive.json").exists());
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("unarchive_stdout", stdout(&output));
    });
}

#[test]
fn archive_refuses_a_package_other_packages_depend_on_unless_forced() {
    let repo = basic_monorepo();
    let dir = repo.path();

    let refused = run_turbo(dir, &["archive", "util"]);
    assert_failure(&refused);
    let message = stderr(&refused);
    assert!(message.contains("`my-app`"), "{message}");
    assert!(message.contains("--force"), "{message}");
    assert!(dir.join("packages/util/package.json").is_file());
    assert!(!dir.join("_archived").exists());

    let forced = run_turbo(dir, &["archive", "util", "--force"]);
    assert_success(&forced);
    let warning = stderr(&forced);
    assert!(warning.contains("WARNING"), "{warning}");
    assert!(warning.contains("`my-app`"), "{warning}");
    assert!(!dir.join("packages/util").exists());
    assert!(dir.join("_archived/util/package.json").is_file());
}

#[test]
fn archive_refuses_a_package_the_root_depends_on() {
    let repo = basic_monorepo();
    let dir = repo.path();
    let path = dir.join("package.json");
    let mut package_json = read_json(&path);
    package_json["dependencies"] = json!({ "another": "*" });
    fs::write(&path, package_json.to_string()).unwrap();

    let output = run_turbo(dir, &["archive", "another"]);
    assert_failure(&output);
    assert!(stderr(&output).contains("`//`"), "{}", stderr(&output));
    assert!(dir.join("packages/another/package.json").is_file());
}

#[test]
fn archive_refuses_a_package_a_task_depends_on() {
    let repo = basic_monorepo();
    let dir = repo.path();
    fs::write(
        dir.join("turbo.json"),
        r#"{ "tasks": { "build": { "dependsOn": ["another#dev"] }, "another#dev": {} } }"#,
    )
    .unwrap();

    let output = run_turbo(dir, &["archive", "another"]);
    assert_failure(&output);
    let message = stderr(&output);
    for expected in ["turbo.json", "`build`", "`another#dev`", "`dependsOn`"] {
        assert!(message.contains(expected), "missing {expected}: {message}");
    }
    assert!(dir.join("packages/another/package.json").is_file());
}

#[test]
fn archive_deletes_task_outputs_and_installs_but_keeps_other_files() {
    let repo = basic_monorepo();
    let dir = repo.path();
    let app = dir.join("apps/my-app");
    for (file, contents) in [
        ("banana.txt", "output"),
        ("apple.json", "{}"),
        ("keep.txt", "source"),
        ("node_modules/dep/index.js", "module.exports = 1;"),
        (".turbo/turbo-build.log", "log"),
    ] {
        let path = app.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    assert!(app.join(".env.local").is_file());

    let output = run_turbo(dir, &["archive", "my-app"]);
    assert_success(&output);

    let archived = dir.join("_archived/my-app");
    for removed in ["banana.txt", "apple.json", "node_modules", ".turbo"] {
        assert!(!archived.join(removed).exists(), "{removed} should be gone");
    }
    for kept in ["keep.txt", ".env.local", "package.json"] {
        assert!(archived.join(kept).is_file(), "{kept} should survive");
    }
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("archive_with_outputs_stdout", stdout(&output));
    });
}

#[test]
fn git_exclude_lines_are_added_and_removed() {
    let repo = basic_monorepo();
    let dir = repo.path();
    let exclude = git_exclude_file(dir);
    let mut seeded = fs::read_to_string(&exclude).unwrap_or_default();
    seeded.push_str("unrelated-pattern\n");
    fs::write(&exclude, &seeded).unwrap();

    assert_success(&run_turbo(dir, &["archive", "another", "--git-exclude"]));
    assert_eq!(
        fs::read_to_string(&exclude).unwrap(),
        format!("{seeded}/packages/another/\n/_archived/another/\n")
    );
    assert_eq!(
        read_json(&dir.join("_archived/another/.turbo-archive.json"))["gitExclude"],
        json!(["/packages/another/", "/_archived/another/"])
    );
    let status = std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(dir)
        .output()
        .unwrap();
    let status = String::from_utf8_lossy(&status.stdout);
    assert!(!status.contains("_archived"), "{status}");

    assert_success(&run_turbo(dir, &["unarchive", "another"]));
    assert_eq!(fs::read_to_string(&exclude).unwrap(), seeded);
}

#[test]
fn literal_package_json_workspace_entry_is_removed_and_restored() {
    let repo = basic_monorepo();
    let dir = repo.path();
    set_root_workspaces(dir, json!(["apps/**", "packages/util", "packages/another"]));

    assert_success(&run_turbo(dir, &["archive", "another"]));
    assert_eq!(
        read_json(&dir.join("package.json"))["workspaces"],
        json!(["apps/**", "packages/util"])
    );
    assert_eq!(
        read_json(&dir.join("_archived/another/.turbo-archive.json"))["workspaceEntry"],
        json!({ "file": "package.json", "entry": "packages/another" })
    );
    assert_eq!(package_paths(dir), ["apps/my-app", "packages/util"]);

    assert_success(&run_turbo(dir, &["unarchive", "another"]));
    assert_eq!(
        read_json(&dir.join("package.json"))["workspaces"],
        json!(["apps/**", "packages/util", "packages/another"])
    );
    assert_eq!(
        package_paths(dir),
        ["packages/another", "apps/my-app", "packages/util"]
    );
}

#[test]
fn pnpm_workspace_yaml_entry_round_trips_with_comments() {
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path();
    let files = [
        (
            "package.json",
            r#"{ "name": "root", "private": true, "packageManager": "pnpm@9.0.0" }"#,
        ),
        (
            "pnpm-workspace.yaml",
            "# workspace packages\npackages:\n  # applications\n  - \"apps/*\"\n  - \
             packages/lib\n\ncatalog:\n  react: ^19.0.0\n",
        ),
        ("turbo.json", r#"{ "tasks": { "build": {} } }"#),
        ("apps/web/package.json", r#"{ "name": "web" }"#),
        ("packages/lib/package.json", r#"{ "name": "lib" }"#),
    ];
    for (file, contents) in files {
        let path = dir.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    setup::setup_git(dir).unwrap();

    assert_success(&run_turbo(dir, &["archive", "lib"]));
    assert_eq!(
        fs::read_to_string(dir.join("pnpm-workspace.yaml")).unwrap(),
        "# workspace packages\npackages:\n  # applications\n  - \"apps/*\"\n\ncatalog:\n  react: \
         ^19.0.0\n"
    );
    assert_eq!(
        read_json(&dir.join("_archived/lib/.turbo-archive.json"))["workspaceEntry"],
        json!({ "file": "pnpm-workspace.yaml", "entry": "packages/lib" })
    );
    assert_eq!(package_paths(dir), ["apps/web"]);

    assert_success(&run_turbo(dir, &["unarchive", "lib"]));
    assert_eq!(
        fs::read_to_string(dir.join("pnpm-workspace.yaml")).unwrap(),
        "# workspace packages\npackages:\n  - \"packages/lib\"\n  # applications\n  - \
         \"apps/*\"\n\ncatalog:\n  react: ^19.0.0\n"
    );
    assert_eq!(package_paths(dir), ["packages/lib", "apps/web"]);
}

#[test]
fn archive_refuses_when_workspace_globs_would_still_match() {
    let repo = basic_monorepo();
    let dir = repo.path();
    set_root_workspaces(dir, json!(["apps/**", "packages/**", "_archived/*"]));

    let output = run_turbo(dir, &["archive", "another"]);
    assert_failure(&output);
    assert!(
        stderr(&output).contains("!_archived/**"),
        "{}",
        stderr(&output)
    );
    assert!(dir.join("packages/another/package.json").is_file());
}

#[test]
fn archive_refuses_a_package_that_contains_another_package() {
    let repo = basic_monorepo();
    let dir = repo.path();
    let nested = dir.join("packages/another/nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("package.json"), r#"{ "name": "nested" }"#).unwrap();

    let output = run_turbo(dir, &["archive", "another"]);
    assert_failure(&output);
    assert!(stderr(&output).contains("`nested`"), "{}", stderr(&output));
    assert!(dir.join("packages/another/package.json").is_file());
    assert!(!dir.join("_archived").exists());
}

#[test]
fn archive_and_unarchive_report_missing_and_repeated_targets() {
    let repo = basic_monorepo();
    let dir = repo.path();

    let missing = run_turbo(dir, &["archive", "nope"]);
    assert_failure(&missing);
    assert!(
        stderr(&missing).contains("not found"),
        "{}",
        stderr(&missing)
    );

    let never_archived = run_turbo(dir, &["unarchive", "another"]);
    assert_failure(&never_archived);
    assert!(
        stderr(&never_archived).contains("not archived"),
        "{}",
        stderr(&never_archived)
    );

    assert_success(&run_turbo(dir, &["archive", "another"]));
    let twice = run_turbo(dir, &["archive", "another"]);
    assert_failure(&twice);
    assert!(
        stderr(&twice).contains("already archived"),
        "{}",
        stderr(&twice)
    );
}

#[test]
fn unarchive_refuses_to_overwrite_an_existing_directory() {
    let repo = basic_monorepo();
    let dir = repo.path();
    assert_success(&run_turbo(dir, &["archive", "another"]));
    fs::create_dir_all(dir.join("packages/another")).unwrap();

    let output = run_turbo(dir, &["unarchive", "another"]);
    assert_failure(&output);
    assert!(
        stderr(&output).contains("already exists"),
        "{}",
        stderr(&output)
    );
    assert!(dir.join("_archived/another/package.json").is_file());
}
