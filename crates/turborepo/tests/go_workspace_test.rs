//! End-to-end tests for experimental Go workspace support.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path};

use common::setup;

const AMBIENT_GO_ENV: &[&str] = &[
    "GO111MODULE",
    "GOARCH",
    "GOENV",
    "GOEXPERIMENT",
    "GOFLAGS",
    "GOOS",
    "GOTOOLCHAIN",
    "GOWORK",
];

fn go_available() -> bool {
    let available = which::which("go").is_ok();
    if !available {
        eprintln!("skipping: go is not on PATH");
    }
    available
}

fn setup_go_pure_workspace(dir: &Path) {
    setup::copy_fixture("go_pure_workspace", dir).unwrap();
    setup::setup_git(dir).unwrap();
    assert!(
        !dir.join("package.json").exists(),
        "the pure Go fixture must have no package.json"
    );
}

fn setup_go_monorepo(dir: &Path) {
    setup::setup_integration_test(dir, "go_monorepo", "npm@10.5.0", false).unwrap();
}

fn run_turbo(dir: &Path, args: &[&str]) -> std::process::Output {
    run_turbo_with_env(dir, args, &[])
}

fn run_turbo_with_env(
    dir: &Path,
    args: &[&str],
    environment: &[(&str, &str)],
) -> std::process::Output {
    let config_dir = tempfile::tempdir().expect("failed to create config tempdir");
    let mut command = common::turbo_command(dir);
    for name in AMBIENT_GO_ENV {
        command.env_remove(name);
    }
    command
        .env("GOENV", "off")
        .env("GOTOOLCHAIN", "local")
        .env("TURBO_CONFIG_DIR_PATH", config_dir.path());
    for (name, value) in environment {
        command.env(name, value);
    }
    command
        .args(args)
        .output()
        .expect("failed to execute turbo")
}

fn assert_command_success(output: &std::process::Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn package_names(dir: &Path) -> Vec<String> {
    let output = run_turbo(dir, &["ls", "--output=json"]);
    assert_command_success(&output, "turbo ls");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("ls emits JSON");
    json["packages"]["items"]
        .as_array()
        .expect("packages items")
        .iter()
        .map(|package| package["name"].as_str().expect("name").to_string())
        .collect()
}

fn query_packages(dir: &Path) -> serde_json::Value {
    let output = run_turbo(
        dir,
        &[
            "query",
            "query { packages { items { name directDependencies { items { name } } } } }",
        ],
    );
    assert_command_success(&output, "turbo query");
    serde_json::from_slice(&output.stdout).expect("query emits JSON")
}

fn task_hash(dir: &Path, package: &str, task: &str) -> String {
    task_hash_with_env(dir, package, task, &[])
}

fn task_hash_with_env(
    dir: &Path,
    package: &str,
    task: &str,
    environment: &[(&str, &str)],
) -> String {
    let filter = format!("--filter={package}");
    let output = run_turbo_with_env(dir, &["run", task, &filter, "--dry-run=json"], environment);
    assert_command_success(&output, "task dry run");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("dry run emits JSON");
    let task_id = format!("{package}#{task}");
    json["tasks"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|task| task["taskId"] == task_id))
        .and_then(|task| task["hash"].as_str())
        .expect("task has a hash")
        .to_string()
}

#[test]
fn test_pure_go_workspace_lists_modules() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());

    let names = package_names(tempdir.path());
    assert!(
        names.contains(&"example.com/api".to_string()),
        "names: {names:?}"
    );
    assert!(
        names.contains(&"example.com/lib".to_string()),
        "names: {names:?}"
    );
}

#[test]
fn test_mixed_go_workspace_lists_js_and_go_packages() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    setup_go_monorepo(tempdir.path());

    let names = package_names(tempdir.path());
    assert!(names.contains(&"js-pkg".to_string()), "names: {names:?}");
    assert!(
        names.contains(&"example.com/api".to_string()),
        "names: {names:?}"
    );
    assert!(
        names.contains(&"example.com/lib".to_string()),
        "names: {names:?}"
    );
}

#[test]
fn test_mixed_workspace_executes_native_go_build() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_monorepo(tempdir.path());
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=example.com/api"],
    );
    assert_command_success(&output, "mixed repository Go build");
    assert!(
        tempdir
            .path()
            .join("apps/api/dist")
            .join(if cfg!(windows) { "api.exe" } else { "api" })
            .exists()
    );
}

#[test]
fn test_disabled_go_workspace_does_not_invoke_go() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_monorepo(tempdir.path());
    let turbo_json = tempdir.path().join("turbo.json");
    let contents = fs::read_to_string(&turbo_json).unwrap();
    fs::write(
        &turbo_json,
        contents.replace(
            "\"experimentalGoWorkspaces\": true",
            "\"experimentalGoWorkspaces\": false",
        ),
    )
    .unwrap();

    let output = run_turbo_with_env(tempdir.path(), &["ls", "--output=json"], &[("PATH", "")]);
    assert_command_success(&output, "disabled Go workspace listing without tools");
    let names = serde_json::from_slice::<serde_json::Value>(&output.stdout).expect("ls emits JSON")
        ["packages"]["items"]
        .as_array()
        .expect("package items")
        .iter()
        .filter_map(|package| package["name"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["js-pkg"]);
}

#[test]
fn test_go_workspace_query_reports_internal_dependencies() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());

    let query = query_packages(tempdir.path());
    let packages = query["data"]["packages"]["items"]
        .as_array()
        .expect("packages array");
    let api = packages
        .iter()
        .find(|package| package["name"] == "example.com/api")
        .expect("api package");
    let dependencies: Vec<&str> = api["directDependencies"]["items"]
        .as_array()
        .expect("dependencies")
        .iter()
        .filter_map(|dependency| dependency["name"].as_str())
        .collect();
    assert_eq!(dependencies, ["example.com/lib"]);
}

#[test]
fn test_go_filter_by_module_path() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &["ls", "--output=json", "--filter=example.com/lib"],
    );
    assert_command_success(&output, "filtered ls");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("ls emits JSON");
    let names = json["packages"]["items"]
        .as_array()
        .expect("packages items")
        .iter()
        .map(|package| package["name"].as_str().expect("name").to_string())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["example.com/lib".to_string()]);
}

#[test]
fn test_disabled_go_workspace_points_to_feature_flag() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    setup_go_monorepo(tempdir.path());
    let turbo_json = tempdir.path().join("turbo.json");
    let contents = fs::read_to_string(&turbo_json).unwrap();
    let updated = contents.replace(
        "\"experimentalGoWorkspaces\": true",
        "\"experimentalGoWorkspaces\": false",
    );
    fs::write(&turbo_json, updated).unwrap();

    let output = run_turbo(tempdir.path(), &["ls", "--filter=example.com/api"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("experimentalGoWorkspaces"),
        "expected disabled-flag guidance, stderr:\n{stderr}"
    );
}

#[test]
fn test_pure_go_workspace_has_no_package_json() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());

    let output = run_turbo(tempdir.path(), &["ls"]);
    assert_command_success(&output, "turbo ls");
    assert!(
        !tempdir.path().join("package.json").exists(),
        "turbo must not create a package.json for a pure Go workspace"
    );
}

#[test]
fn test_native_go_tasks_execute_and_restore_cached_binary() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());

    for task in ["test", "vet"] {
        let output = run_turbo(
            tempdir.path(),
            &["run", task, "--filter=go-workspace", "--log-order=grouped"],
        );
        assert_command_success(&output, &format!("workspace {task}"));
    }

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--filter=example.com/api",
            "--log-order=grouped",
        ],
    );
    assert_command_success(&output, "first Go build");
    let binary =
        tempdir
            .path()
            .join("apps/api/dist")
            .join(if cfg!(windows) { "api.exe" } else { "api" });
    assert!(binary.exists(), "native build must produce {binary:?}");

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--filter=example.com/api",
            "--log-order=grouped",
        ],
    );
    assert_command_success(&output, "cached Go build");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("FULL TURBO"),
        "second build must hit cache: {output:?}"
    );

    fs::remove_file(&binary).unwrap();
    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--filter=example.com/api",
            "--log-order=grouped",
        ],
    );
    assert_command_success(&output, "restored Go build");
    assert!(binary.exists(), "cache hit must restore the binary");

    let output = run_turbo(
        tempdir.path(),
        &["run", "run", "--filter=example.com/api", "--", "argument"],
    );
    assert_command_success(&output, "native Go run with pass-through argument");
}

#[test]
fn test_go_task_contracts_format_overrides_and_failures() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=example.com/api", "--dry-run=json"],
    );
    assert_command_success(&output, "Go build dry run");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("dry run emits JSON");
    let build = json["tasks"]
        .as_array()
        .and_then(|tasks| {
            tasks
                .iter()
                .find(|task| task["taskId"] == "example.com/api#build")
        })
        .expect("api build task");
    let definition = &build["resolvedTaskDefinition"];
    assert_eq!(definition["cache"], true);
    assert!(
        definition["inputs"]
            .as_array()
            .is_some_and(|inputs| inputs.iter().any(|input| input == "../../go.work"))
    );
    assert!(
        definition["outputs"]
            .as_array()
            .is_some_and(|outputs| outputs.iter().any(|output| output == "dist/api"))
    );
    assert!(
        definition["env"]
            .as_array()
            .is_some_and(|env| env.iter().any(|name| name == "GOOS"))
    );
    assert!(
        build["hashOfExternalDependencies"]
            .as_str()
            .is_some_and(|hash| !hash.is_empty())
    );

    fs::write(
        tempdir.path().join("packages/lib/lib.go"),
        "package lib\nfunc   Greet( ){ }\n",
    )
    .unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["run", "format", "--filter=example.com/lib"],
    );
    assert_command_success(&output, "native Go format");
    assert_eq!(
        fs::read_to_string(tempdir.path().join("packages/lib/lib.go")).unwrap(),
        "package lib\n\nfunc Greet() {}\n"
    );

    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": {
    "experimentalGoWorkspaces": true,
    "experimentalTaskCommand": true
  },
  "tasks": {
    "build": { "command": { "go": ["go", "version"] } }
  }
}"#,
    )
    .unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=example.com/api"],
    );
    assert_command_success(&output, "overridden Go build");
    assert!(String::from_utf8_lossy(&output.stdout).contains("go version go"));
    assert!(
        !tempdir.path().join("apps/api/dist/api").exists(),
        "the native build must not shadow the override"
    );

    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": {
    "experimentalGoWorkspaces": true,
    "experimentalTaskCommand": true
  },
  "tasks": {}
}"#,
    )
    .unwrap();
    fs::write(
        tempdir.path().join("packages/lib/lib_test.go"),
        "package lib\n\nimport \"testing\"\n\nfunc TestFailure(t *testing.T) { \
         t.Fatal(\"intentional failure\") }\n",
    )
    .unwrap();
    let output = run_turbo(tempdir.path(), &["run", "test", "--filter=example.com/lib"]);
    assert!(
        !output.status.success(),
        "a failing Go test must fail the task"
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(combined.contains("intentional failure"), "{combined}");
}

#[test]
fn test_go_hash_is_checkout_stable_and_environment_sensitive() {
    if !go_available() {
        return;
    }
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(first.path());
    setup_go_pure_workspace(second.path());

    let first_hash = task_hash(first.path(), "example.com/api", "build");
    assert_eq!(
        first_hash,
        task_hash(second.path(), "example.com/api", "build"),
        "equivalent checkout roots must produce the same hash"
    );
    assert_ne!(
        first_hash,
        task_hash_with_env(
            first.path(),
            "example.com/api",
            "build",
            &[("GOARCH", "386")],
        ),
        "target architecture must affect the hash"
    );
    assert_ne!(
        first_hash,
        task_hash_with_env(
            first.path(),
            "example.com/api",
            "build",
            &[("GOFLAGS", "-tags=turbo_hash_test")],
        ),
        "Go build flags must affect the hash"
    );
    assert_eq!(
        first_hash,
        task_hash_with_env(
            first.path(),
            "example.com/api",
            "build",
            &[("GOPROXY", "https://credentials@example.invalid")],
        ),
        "proxy and credential configuration must not affect the hash"
    );
}

#[test]
fn test_go_definition_sums_invalidate_hashes() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    let package = "example.com/api";
    let original = task_hash(tempdir.path(), package, "build");
    let empty_sum = "h1:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=";

    fs::write(
        tempdir.path().join("packages/lib/go.sum"),
        format!("example.net/unused v1.0.0/go.mod {empty_sum}\n"),
    )
    .unwrap();
    let module_sum = task_hash(tempdir.path(), package, "build");
    assert_ne!(original, module_sum, "module go.sum must affect dependents");

    fs::write(
        tempdir.path().join("go.work.sum"),
        format!("example.net/workspace v1.0.0/go.mod {empty_sum}\n"),
    )
    .unwrap();
    assert_ne!(
        module_sum,
        task_hash(tempdir.path(), package, "build"),
        "go.work.sum must affect Go task hashes"
    );
}

#[test]
fn test_go_task_hash_tracks_dependencies_but_not_unrelated_files() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    let original = task_hash(tempdir.path(), "example.com/api", "build");

    fs::write(tempdir.path().join("unrelated.txt"), "unrelated\n").unwrap();
    assert_eq!(
        original,
        task_hash(tempdir.path(), "example.com/api", "build"),
        "repository-root siblings outside the module must not affect its hash"
    );

    fs::write(
        tempdir.path().join("packages/lib/lib.go"),
        "package lib\n\nfunc Greet() { println(\"changed\") }\n",
    )
    .unwrap();
    assert_ne!(
        original,
        task_hash(tempdir.path(), "example.com/api", "build"),
        "internal dependency sources must affect dependent module hashes"
    );
}

#[test]
fn test_affected_go_tasks_follow_internal_module_relationships() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    fs::write(
        tempdir.path().join("packages/lib/lib.go"),
        "package lib\n\nfunc Greet() { println(\"affected\") }\n",
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks(tasks: [\"build\"]) { items { name package { name } } } }",
        ],
    );
    assert_command_success(&output, "Go affected task query");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("affected query JSON");
    let tasks = json["data"]["affectedTasks"]["items"]
        .as_array()
        .expect("affected tasks");
    for package in ["example.com/lib", "example.com/api"] {
        assert!(
            tasks
                .iter()
                .any(|task| task["name"] == "build" && task["package"]["name"] == package),
            "{package}#build must be affected: {tasks:?}"
        );
    }
}

#[test]
fn test_affected_go_tasks_do_not_cross_independent_modules() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    let independent = tempdir.path().join("tools/independent");
    fs::create_dir_all(&independent).unwrap();
    fs::write(
        independent.join("go.mod"),
        "module example.com/independent\n\ngo 1.22\n",
    )
    .unwrap();
    fs::write(independent.join("main.go"), "package independent\n").unwrap();
    fs::write(
        tempdir.path().join("go.work"),
        "go 1.22\n\nuse (\n\t./apps/api\n\t./packages/lib\n\t./tools/independent\n)\n",
    )
    .unwrap();
    common::git(tempdir.path(), &["add", "."]);
    common::git(
        tempdir.path(),
        &["commit", "-m", "add independent module", "--quiet"],
    );

    fs::write(
        independent.join("main.go"),
        "package independent\n\nconst Changed = true\n",
    )
    .unwrap();
    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks(tasks: [\"build\"]) { items { name package { name } } } }",
        ],
    );
    assert_command_success(&output, "independent Go affected task query");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("affected query JSON");
    let tasks = json["data"]["affectedTasks"]["items"]
        .as_array()
        .expect("affected tasks");
    assert!(tasks.iter().any(|task| {
        task["name"] == "build" && task["package"]["name"] == "example.com/independent"
    }));
    for package in ["example.com/api", "example.com/lib"] {
        assert!(
            !tasks
                .iter()
                .any(|task| task["name"] == "build" && task["package"]["name"] == package),
            "{package} must remain unaffected: {tasks:?}"
        );
    }
}

#[test]
fn test_query_exposes_go_tasks_and_package_exclusion() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    let query = |dir: &Path| {
        let output = run_turbo(
            dir,
            &[
                "query",
                "query { package(name: \"example.com/api\") { tasks { items { name command } } } }",
            ],
        );
        assert_command_success(&output, "Go task query");
        serde_json::from_slice::<serde_json::Value>(&output.stdout).expect("query JSON")
    };
    let tasks = query(tempdir.path());
    let items = tasks["data"]["package"]["tasks"]["items"]
        .as_array()
        .expect("task items");
    assert!(items.iter().any(|task| {
        task["name"] == "build"
            && task["command"]
                .as_str()
                .is_some_and(|command| command.contains("go build -o dist/api"))
    }));
    assert!(items.iter().any(|task| task["name"] == "run"));
    assert!(items.iter().any(|task| task["name"] == "vet"));

    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": {
    "experimentalGoWorkspaces": true,
    "experimentalTaskCommand": true
  },
  "tasks": {
    "build": { "command": { "go": ["go", "version"] } }
  }
}"#,
    )
    .unwrap();
    let tasks = query(tempdir.path());
    let build = tasks["data"]["package"]["tasks"]["items"]
        .as_array()
        .expect("task items")
        .iter()
        .find(|task| task["name"] == "build")
        .expect("overridden build");
    assert_eq!(build["command"], "go version");

    fs::write(
        tempdir.path().join("apps/api/turbo.json"),
        r#"{
  "extends": ["//"],
  "tasks": { "build": { "extends": false } }
}"#,
    )
    .unwrap();
    let tasks = query(tempdir.path());
    assert!(
        !tasks["data"]["package"]["tasks"]["items"]
            .as_array()
            .expect("task items")
            .iter()
            .any(|task| task["name"] == "build")
    );
}

#[test]
fn test_query_exposes_go_external_resolution_dependents() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { externalDependencies { items { name internalDependents { items { name } } } \
             } }",
        ],
    );
    assert_command_success(&output, "Go external dependency query");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("query emits JSON");
    let packages = json["data"]["externalDependencies"]["items"]
        .as_array()
        .expect("external dependencies");
    let toolchain = packages
        .iter()
        .find(|package| package["name"] == "go")
        .expect("Go toolchain identity");
    let dependents = toolchain["internalDependents"]["items"]
        .as_array()
        .expect("toolchain dependents");
    for package in ["example.com/api", "example.com/lib", "go-workspace"] {
        assert!(
            dependents
                .iter()
                .any(|dependent| dependent["name"] == package),
            "{package} must report the Go toolchain dependency: {dependents:?}"
        );
    }
}

#[test]
fn test_go_prune_produces_valid_buildable_workspace() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    let independent = tempdir.path().join("tools/independent");
    fs::create_dir_all(&independent).unwrap();
    fs::write(
        independent.join("go.mod"),
        "module example.com/independent\n\ngo 1.22\n",
    )
    .unwrap();
    fs::write(independent.join("independent.go"), "package independent\n").unwrap();
    fs::write(
        tempdir.path().join("go.work"),
        "go 1.22\n\nuse (\n\t./apps/api\n\t./packages/lib\n\t./tools/independent\n)\n",
    )
    .unwrap();
    let output = run_turbo(tempdir.path(), &["prune", "example.com/api"]);
    assert_command_success(&output, "Go prune");

    let pruned = tempdir.path().join("out");
    let go_work = fs::read_to_string(pruned.join("go.work")).expect("pruned go.work");
    assert!(go_work.contains("./apps/api"));
    assert!(go_work.contains("./packages/lib"));
    assert!(!go_work.contains("./tools/independent"));
    assert!(pruned.join("packages/lib/go.mod").exists());
    assert!(!pruned.join("tools/independent").exists());

    let output = std::process::Command::new("go")
        .args(["test", "./..."])
        .env("GOENV", "off")
        .env("GOTOOLCHAIN", "local")
        .current_dir(pruned.join("apps/api"))
        .output()
        .expect("go test runs in pruned module");
    assert_command_success(&output, "pruned Go module tests");

    let output = run_turbo(&pruned, &["run", "build", "--filter=example.com/api"]);
    assert_command_success(&output, "native build in pruned workspace");
}
