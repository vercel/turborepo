//! End-to-end tests for experimental Go workspace support.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path};
#[cfg(unix)]
use std::{
    process::{Child, Stdio},
    time::Duration,
};

use common::setup;

const AMBIENT_GO_ENV: &[&str] = &[
    "AR",
    "CC",
    "CGO_CFLAGS",
    "CGO_CPPFLAGS",
    "CGO_CXXFLAGS",
    "CGO_ENABLED",
    "CGO_FFLAGS",
    "CGO_LDFLAGS",
    "CXX",
    "GCCGO",
    "GO111MODULE",
    "GO386",
    "GOAMD64",
    "GOARCH",
    "GOARM",
    "GOARM64",
    "GOCACHE",
    "GOCACHEPROG",
    "GODEBUG",
    "GOENV",
    "GOEXPERIMENT",
    "GOFIPS140",
    "GOFLAGS",
    "GOMIPS",
    "GOMIPS64",
    "GOMODCACHE",
    "GOOS",
    "GOPPC64",
    "GORISCV64",
    "GOTOOLCHAIN",
    "GOWASM",
    "GOWORK",
    "PKG_CONFIG",
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

fn setup_go_e2e_workspace(dir: &Path) {
    setup::copy_fixture("go_e2e_workspace", dir).unwrap();
    setup::setup_git(dir).unwrap();
}

fn run_turbo(dir: &Path, args: &[&str]) -> std::process::Output {
    run_turbo_with_env(dir, args, &[])
}

fn run_turbo_with_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> std::process::Output {
    let config_dir = tempfile::tempdir().expect("failed to create config tempdir");
    let go_cache_dir = tempfile::tempdir().expect("failed to create Go cache tempdir");
    let mut command = common::turbo_command(dir);
    for name in AMBIENT_GO_ENV {
        command.env_remove(name);
    }
    command
        .env("GOCACHE", go_cache_dir.path())
        .env("GOENV", "off")
        .env("GOTOOLCHAIN", "local")
        .env("TURBO_CONFIG_DIR_PATH", config_dir.path());
    command.envs(env.iter().copied());
    command
        .args(args)
        .output()
        .expect("failed to execute turbo")
}

fn run_go(dir: &Path, args: &[&str]) -> std::process::Output {
    let cache = tempfile::tempdir().expect("failed to create Go cache tempdir");
    let mut command = std::process::Command::new("go");
    for name in AMBIENT_GO_ENV {
        command.env_remove(name);
    }
    command
        .args(args)
        .env("GOCACHE", cache.path())
        .env("GOENV", "off")
        .env("GOTOOLCHAIN", "local")
        .current_dir(dir)
        .output()
        .expect("failed to execute go")
}

fn assert_command_success(output: &std::process::Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_go_build_cache_result(
    dir: &Path,
    environment: &[(&str, &str)],
    expected: &str,
    context: &str,
) {
    let output = run_turbo_with_env(
        dir,
        &[
            "run",
            "build",
            "--filter=example.com/api",
            "--log-order=grouped",
        ],
        environment,
    );
    assert_command_success(&output, context);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains(expected),
        "{context} must report {expected:?}\noutput:\n{combined}"
    );
}

#[cfg(unix)]
fn go_version_shim_path(dir: &Path) -> String {
    use std::os::unix::fs::PermissionsExt;

    let real_go = which::which("go").expect("go is available");
    let shim_dir = dir.join("go-version-shim");
    fs::create_dir_all(&shim_dir).unwrap();
    let quoted_go = real_go.to_string_lossy().replace('\'', "'\"'\"'");
    let shim = shim_dir.join("go");
    fs::write(
        &shim,
        format!(
            concat!(
                "#!/bin/sh\n",
                "if [ \"$1\" = version ]; then\n",
                "  echo 'go version go1.99.0 turbo/e2e'\n",
                "  exit 0\n",
                "fi\n",
                "exec '{}' \"$@\"\n",
            ),
            quoted_go
        ),
    )
    .unwrap();
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();

    let mut paths = vec![shim_dir];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(paths)
        .expect("PATH can include the Go version shim")
        .to_string_lossy()
        .into_owned()
}

#[cfg(unix)]
fn wait_for_path(path: &Path, timeout: Duration) -> bool {
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    path.exists()
}

#[cfg(unix)]
struct GoWatchGuard(Option<Child>);

#[cfg(unix)]
impl GoWatchGuard {
    fn spawn(dir: &Path) -> Self {
        use std::os::unix::process::CommandExt;

        let mut command = std::process::Command::new(assert_cmd::cargo::cargo_bin("turbo"));
        command.process_group(0);
        for name in AMBIENT_GO_ENV {
            command.env_remove(name);
        }
        for name in common::ambient_turbo_env_keys() {
            command.env_remove(name);
        }
        command
            .args(["watch", "build"])
            .env("GOCACHE", dir.join(".cache/go-build"))
            .env("GOMODCACHE", dir.join(".cache/go-mod"))
            .env("GOENV", "off")
            .env("GOTOOLCHAIN", "local")
            .env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
            .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
            .env("TURBO_PRINT_VERSION_DISABLED", "1")
            .env("DO_NOT_TRACK", "1")
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        Self(Some(command.spawn().expect("failed to spawn turbo watch")))
    }
}

#[cfg(unix)]
impl Drop for GoWatchGuard {
    fn drop(&mut self) {
        use nix::{
            sys::signal::{self, Signal},
            unistd::Pid,
        };

        let Some(mut child) = self.0.take() else {
            return;
        };
        let group = Pid::from_raw(-(child.id() as i32));
        let _ = signal::kill(group, Signal::SIGTERM);
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_secs(10) {
            match child.try_wait() {
                Ok(Some(_)) | Err(_) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
        let _ = signal::kill(group, Signal::SIGKILL);
        let _ = child.wait();
    }
}

fn dry_run_task(output: &std::process::Output, task_id: &str) -> serde_json::Value {
    assert_command_success(output, "Go task dry run");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("dry run emits JSON");
    json["tasks"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|task| task["taskId"] == task_id))
        .cloned()
        .unwrap_or_else(|| panic!("{task_id} in task graph"))
}

fn task_hash(dir: &Path, package: &str, task: &str) -> String {
    let output = run_turbo(
        dir,
        &[
            "run",
            task,
            &format!("--filter={package}"),
            "--dry-run=json",
        ],
    );
    dry_run_task(&output, &format!("{package}#{task}"))["hash"]
        .as_str()
        .expect("task has a hash")
        .to_string()
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

fn package_task_names(dir: &Path, package: &str) -> Vec<String> {
    let query =
        format!("query {{ package(name: \"{package}\") {{ tasks {{ items {{ name }} }} }} }}");
    let output = run_turbo(dir, &["query", &query]);
    assert_command_success(&output, "Go task catalog query");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("query emits JSON");
    json["data"]["package"]["tasks"]["items"]
        .as_array()
        .expect("task items")
        .iter()
        .map(|task| task["name"].as_str().expect("task name").to_string())
        .collect()
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
fn test_mixed_workspace_executes_and_caches_javascript_and_go_builds() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    setup_go_monorepo(tempdir.path());
    let args = ["run", "build", "--log-order=grouped"];
    let output = run_turbo(tempdir.path(), &args);
    assert_command_success(&output, "mixed JavaScript and Go build");
    assert!(
        tempdir.path().join("packages/js-pkg/dist/out.txt").exists(),
        "the JavaScript package must produce its declared output"
    );
    assert!(
        tempdir
            .path()
            .join("apps/api/dist")
            .join(if cfg!(windows) { "api.exe" } else { "api" })
            .exists(),
        "the Go module must produce its native executable"
    );

    let output = run_turbo(tempdir.path(), &args);
    assert_command_success(&output, "warm mixed JavaScript and Go build");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "equivalent mixed tasks must hit cache:\n{stdout}"
    );
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
fn test_go_prune_produces_minimal_valid_workspace() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    let unused = tempdir.path().join("tools/unused");
    fs::create_dir_all(&unused).unwrap();
    fs::write(
        unused.join("go.mod"),
        "module example.com/unused\n\ngo 1.22\n",
    )
    .unwrap();
    fs::write(unused.join("unused.go"), "package unused\n").unwrap();
    fs::write(
        tempdir.path().join("go.work"),
        "go 1.22\n\nuse (\n\t./tools/unused\n\t./packages/lib\n\t./apps/api\n)\n",
    )
    .unwrap();
    let checksum = "h1:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=";
    let work_sum = format!("example.net/workspace v1.0.0/go.mod {checksum}\n");
    fs::write(tempdir.path().join("go.work.sum"), &work_sum).unwrap();
    fs::write(
        tempdir.path().join("packages/lib/go.sum"),
        format!("example.net/module v1.0.0/go.mod {checksum}\n"),
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["prune", "example.com/api", "--docker"]);
    assert_command_success(&output, "Go prune");
    let out = tempdir.path().join("out");
    let full = out.join("full");
    let json = out.join("json");
    let expected_work = "go 1.22\n\nuse (\n\t./apps/api\n\t./packages/lib\n)\n";
    assert_eq!(
        fs::read_to_string(full.join("go.work")).unwrap(),
        expected_work
    );
    assert_eq!(
        fs::read_to_string(json.join("go.work")).unwrap(),
        expected_work
    );
    assert_eq!(
        fs::read_to_string(full.join("go.work.sum")).unwrap(),
        work_sum
    );
    assert_eq!(
        fs::read_to_string(json.join("go.work.sum")).unwrap(),
        work_sum
    );
    for root in [&full, &json] {
        assert!(root.join("apps/api/go.mod").exists());
        assert!(root.join("packages/lib/go.mod").exists());
        assert!(root.join("packages/lib/go.sum").exists());
        assert!(!root.join("tools/unused").exists());
    }

    for args in [
        &["work", "edit", "-json"][..],
        &["list", "-m", "all"][..],
        &["test", "./apps/api/...", "./packages/lib/..."][..],
    ] {
        let output = run_go(&full, args);
        assert!(
            output.status.success(),
            "go {args:?} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    for task in ["build", "test"] {
        let output = run_turbo(
            &full,
            &[
                "run",
                task,
                "--filter=example.com/api",
                "--log-order=grouped",
            ],
        );
        assert_command_success(&output, &format!("pruned native Go {task} task"));
    }
    assert!(
        full.join("apps/api/dist")
            .join(if cfg!(windows) { "api.exe" } else { "api" })
            .exists(),
        "the pruned native build must produce its executable"
    );
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
fn test_enabled_go_workspace_reports_missing_go_executable() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());

    let output = run_turbo_with_env(tempdir.path(), &["ls"], &[("PATH", "")]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Go is required for experimental Go workspaces")
            && stderr.contains("Install Go 1.22 or newer")
            && stderr.contains("PATH"),
        "missing Go diagnostic must identify the requirement and remediation: {stderr}"
    );
    assert!(!stderr.contains("package manager"), "{stderr}");
}

#[test]
fn test_invalid_go_workspace_reports_repair_without_javascript_fallback() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    fs::write(
        tempdir.path().join("go.work"),
        "go 1.22\n\nunsupported ./apps/api\n",
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["ls"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("`go work edit -json` failed")
            && stderr.contains("Repair the repository-root go.work"),
        "invalid workspace must have focused remediation: {stderr}"
    );
    assert!(!stderr.contains("package manager"), "{stderr}");
    assert!(!stderr.contains("failed to parse"), "{stderr}");
}

#[test]
fn test_go_and_javascript_package_name_collision_is_actionable() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_monorepo(tempdir.path());
    fs::write(
        tempdir.path().join("apps/api/go.mod"),
        "module js-pkg\n\ngo 1.22\n\nrequire example.com/lib v0.0.0\n\nreplace example.com/lib => \
         ../../packages/lib\n",
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["ls"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("package identity \"js-pkg\" is declared by both")
            && stderr.contains("Rename one package or module"),
        "cross-language identity collision must be actionable: {stderr}"
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
fn test_go_native_tasks_and_workspace_aggregate() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": { "experimentalGoWorkspaces": true },
  "tasks": {}
}"#,
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["run", "test", "--dry-run=json"]);
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("dry run emits JSON");
    assert_eq!(json["tasks"].as_array().map(Vec::len), Some(1));
    let task = dry_run_task(&output, "go-workspace#test");
    assert_eq!(task["command"], "go test ./apps/api/... ./packages/lib/...");
    assert_eq!(task["directory"], "");

    let output = run_turbo(tempdir.path(), &["run", "lint", "--dry-run=json"]);
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("dry run emits JSON");
    assert_eq!(json["tasks"].as_array().map(Vec::len), Some(1));
    let lint = dry_run_task(&output, "go-workspace#lint");
    assert_eq!(lint["command"], "go vet ./apps/api/... ./packages/lib/...");
    assert_eq!(lint["directory"], "");

    let output = run_turbo(
        tempdir.path(),
        &["run", "lint", "--filter=example.com/lib", "--dry-run=json"],
    );
    let lint = dry_run_task(&output, "example.com/lib#lint");
    assert_eq!(lint["command"], "go vet ./...");

    let output = run_turbo(tempdir.path(), &["run", "build", "--dry-run=json"]);
    let build = dry_run_task(&output, "example.com/api#build");
    let executable = if cfg!(windows) { "api.exe" } else { "api" };
    let output_path = format!("dist/{executable}");
    assert_eq!(build["command"], format!("go build -o {output_path} ."));
    assert_eq!(build["resolvedTaskDefinition"]["cache"], true);
    assert!(
        build["resolvedTaskDefinition"]["outputs"]
            .as_array()
            .is_some_and(|outputs| outputs.iter().any(|output| output == &output_path))
    );

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=example.com/lib", "--dry-run=json"],
    );
    let build = dry_run_task(&output, "example.com/lib#build");
    assert_eq!(build["command"], "go build ./...");
    assert_eq!(build["resolvedTaskDefinition"]["cache"], false);

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "dev",
            "--filter=example.com/api",
            "--dry-run=json",
            "--",
            "--port",
            "3000",
        ],
    );
    let dev = dry_run_task(&output, "example.com/api#dev");
    assert_eq!(dev["command"], "go run .");

    let tasks = package_task_names(tempdir.path(), "example.com/api");
    assert!(tasks.iter().any(|task| task == "dev"), "tasks: {tasks:?}");
    assert!(tasks.iter().any(|task| task == "lint"), "tasks: {tasks:?}");
    assert!(!tasks.iter().any(|task| task == "run"), "tasks: {tasks:?}");
    assert!(!tasks.iter().any(|task| task == "vet"), "tasks: {tasks:?}");

    let output = run_turbo(
        tempdir.path(),
        &["run", "run", "--filter=example.com/api", "--dry-run=json"],
    );
    assert!(!output.status.success(), "removed run task must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Could not find task `run` in project"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn test_go_task_hash_tracks_source_but_not_unrelated_siblings() {
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
        tempdir.path().join("apps/api/main.go"),
        "package main\n\nimport \"example.com/lib\"\n\nfunc main() { lib.Greet(); \
         println(\"changed\") }\n",
    )
    .unwrap();
    assert_ne!(
        original,
        task_hash(tempdir.path(), "example.com/api", "build"),
        "module source changes must affect its task hash"
    );
}

#[test]
fn test_go_hash_is_stable_across_equivalent_checkout_roots() {
    if !go_available() {
        return;
    }

    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    setup_go_e2e_workspace(first.path());
    setup_go_e2e_workspace(second.path());

    assert_eq!(
        task_hash(first.path(), "example.com/api", "build"),
        task_hash(second.path(), "example.com/api", "build"),
        "equivalent checkouts with local replacements must produce the same Go task hash"
    );
}

#[test]
fn test_go_cache_invalidates_every_north_star_input() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    setup_go_e2e_workspace(tempdir.path());
    let root = tempdir.path();

    assert_go_build_cache_result(root, &[], "cache miss", "cold Go build");
    assert_go_build_cache_result(root, &[], "FULL TURBO", "unchanged Go build");

    fs::write(
        root.join("tools/independent/independent.go"),
        "package independent\n\nconst Value = \"still-independent\"\n",
    )
    .unwrap();
    assert_go_build_cache_result(root, &[], "FULL TURBO", "unrelated module source change");

    fs::write(
        root.join("apps/api/main.go"),
        r#"package main

import (
	"fmt"

	"example.com/lib"
	"example.net/message"
)

func main() {
	fmt.Println(lib.Value(), message.Value(), "source-changed")
}
"#,
    )
    .unwrap();
    assert_go_build_cache_result(root, &[], "cache miss", "module source change");

    fs::write(
        root.join("packages/lib/lib.go"),
        "package lib\n\nfunc Value() string { return \"dependency-changed\" }\n",
    )
    .unwrap();
    assert_go_build_cache_result(root, &[], "cache miss", "internal dependency change");

    fs::write(
        root.join("third_party/message/message.go"),
        "package message\n\nfunc Value() string { return \"replacement-changed\" }\n",
    )
    .unwrap();
    assert_go_build_cache_result(root, &[], "cache miss", "local replacement change");

    fs::write(
        root.join("apps/api/go.mod"),
        r#"module example.com/api

go 1.22

require (
	example.com/independent v0.0.0
	example.com/lib v0.0.0
	example.net/message v0.0.0
)

replace example.com/independent => ../../tools/independent

replace example.com/lib => ../../packages/lib

replace example.net/message => ../../third_party/message
"#,
    )
    .unwrap();
    assert_go_build_cache_result(root, &[], "cache miss", "module graph change");

    fs::write(
        root.join("tools/independent/independent.go"),
        "package independent\n\nconst Value = \"now-dependent\"\n",
    )
    .unwrap();
    assert_go_build_cache_result(
        root,
        &[],
        "cache miss",
        "newly connected dependency source change",
    );

    fs::write(
        root.join("apps/api/go.sum"),
        "example.org/checksum-only v1.0.1/go.mod h1:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=\n",
    )
    .unwrap();
    assert_go_build_cache_result(root, &[], "cache miss", "external checksum change");

    #[cfg(unix)]
    {
        let path = go_version_shim_path(root);
        assert_go_build_cache_result(
            root,
            &[("PATH", &path)],
            "cache miss",
            "Go compiler version change",
        );
    }

    let go_arch = run_go(root, &["env", "GOARCH"]);
    assert_command_success(&go_arch, "read host Go architecture");
    let target_arch = if String::from_utf8_lossy(&go_arch.stdout).trim() == "386" {
        "amd64"
    } else {
        "386"
    };
    assert_go_build_cache_result(
        root,
        &[("GOARCH", target_arch)],
        "cache miss",
        "Go target architecture change",
    );
    assert_go_build_cache_result(
        root,
        &[("GOFLAGS", "-tags=turbo_cache_invalidation")],
        "cache miss",
        "relevant Go build environment change",
    );
}

#[test]
fn test_go_resolution_sums_invalidate_dependent_task_hashes() {
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
    assert_ne!(
        original, module_sum,
        "a dependency module's go.sum must invalidate dependents"
    );

    fs::write(
        tempdir.path().join("go.work.sum"),
        format!("example.net/workspace v1.0.0/go.mod {empty_sum}\n"),
    )
    .unwrap();
    assert_ne!(
        module_sum,
        task_hash(tempdir.path(), package, "build"),
        "go.work.sum must invalidate Go task hashes"
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

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--affected", "--dry=json"],
        &[("TURBO_SCM_BASE", "HEAD")],
    );
    assert_command_success(&output, "Go --affected dry run");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("affected dry-run JSON");
    let tasks = json["tasks"].as_array().expect("affected tasks");
    for package in ["example.com/lib", "example.com/api"] {
        let task_id = format!("{package}#build");
        assert!(
            tasks.iter().any(|task| task["taskId"] == task_id),
            "{task_id} must be affected: {tasks:?}"
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
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": {
    "experimentalGoWorkspaces": true,
    "experimentalTaskCommand": true,
    "affectedUsingTaskInputs": true
  },
  "tasks": { "build": { "dependsOn": ["^build"] } }
}"#,
    )
    .unwrap();
    common::git(tempdir.path(), &["add", "."]);
    common::git(
        tempdir.path(),
        &["commit", "-m", "add independent module", "--quiet"],
    );

    let api_hash = task_hash(tempdir.path(), "example.com/api", "build");
    let lib_hash = task_hash(tempdir.path(), "example.com/lib", "build");
    let independent_hash = task_hash(tempdir.path(), "example.com/independent", "build");
    fs::write(
        independent.join("main.go"),
        "package independent\n\nconst Changed = true\n",
    )
    .unwrap();
    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks(base: \"HEAD\", tasks: [\"build\"]) { items { name package { \
             name } } } }",
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
    assert_ne!(
        independent_hash,
        task_hash(tempdir.path(), "example.com/independent", "build"),
        "the changed module must be invalidated"
    );
    assert_eq!(
        api_hash,
        task_hash(tempdir.path(), "example.com/api", "build"),
        "an unrelated module must not invalidate the API"
    );
    assert_eq!(
        lib_hash,
        task_hash(tempdir.path(), "example.com/lib", "build"),
        "an unrelated module must not invalidate the library"
    );
}

#[cfg(unix)]
#[test]
fn test_go_watch_rediscovers_workspace_members_with_repository_local_caches() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    let _watch = GoWatchGuard::spawn(tempdir.path());
    let api_binary = tempdir.path().join("apps/api/dist/api");
    assert!(
        wait_for_path(&api_binary, Duration::from_secs(30)),
        "initial Go watch build did not produce {api_binary:?}"
    );

    let worker = tempdir.path().join("apps/worker");
    fs::create_dir_all(&worker).unwrap();
    fs::write(
        worker.join("go.mod"),
        "module example.com/worker\n\ngo 1.22\n",
    )
    .unwrap();
    fs::write(
        worker.join("main.go"),
        "package main\n\nfunc main() { println(\"worker\") }\n",
    )
    .unwrap();
    fs::write(
        tempdir.path().join("go.work"),
        "go 1.22\n\nuse (\n\t./apps/api\n\t./apps/worker\n\t./packages/lib\n)\n",
    )
    .unwrap();
    common::git(
        tempdir.path(),
        &[
            "add",
            "go.work",
            "apps/worker/go.mod",
            "apps/worker/main.go",
        ],
    );
    common::git(
        tempdir.path(),
        &["commit", "-m", "add worker module", "--quiet"],
    );

    let worker_binary = worker.join("dist/worker");
    assert!(
        wait_for_path(&worker_binary, Duration::from_secs(30)),
        "turbo watch did not rediscover and build {worker_binary:?}"
    );
}

#[test]
fn test_go_native_tasks_are_overrideable_and_excludable() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": {
    "experimentalGoWorkspaces": true,
    "experimentalTaskCommand": true
  },
  "tasks": {
    "lint": { "command": { "go": ["go", "version"] } },
    "dev": { "command": { "go": ["go", "env", "GOVERSION"] } }
  }
}"#,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "lint", "--filter=example.com/lib", "--dry-run=json"],
    );
    let lint = dry_run_task(&output, "example.com/lib#lint");
    assert_eq!(lint["command"], "go version");

    let output = run_turbo(
        tempdir.path(),
        &["run", "dev", "--filter=example.com/api", "--dry-run=json"],
    );
    let dev = dry_run_task(&output, "example.com/api#dev");
    assert_eq!(dev["command"], "go env GOVERSION");

    fs::write(
        tempdir.path().join("apps/api/turbo.json"),
        r#"{
  "extends": ["//"],
  "tasks": {
    "dev": { "extends": false }
  }
}"#,
    )
    .unwrap();
    let tasks = package_task_names(tempdir.path(), "example.com/api");
    assert!(tasks.iter().any(|task| task == "lint"), "tasks: {tasks:?}");
    assert!(!tasks.iter().any(|task| task == "vet"), "tasks: {tasks:?}");
    assert!(!tasks.iter().any(|task| task == "run"), "tasks: {tasks:?}");
    assert!(!tasks.iter().any(|task| task == "dev"), "tasks: {tasks:?}");
}

#[test]
fn test_native_go_tasks_execute_cache_restore_and_pass_through_args() {
    if !go_available() {
        return;
    }

    let unfiltered = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(unfiltered.path());
    for task in ["build", "test", "lint"] {
        let output = run_turbo(unfiltered.path(), &["run", task, "--log-order=grouped"]);
        assert_command_success(&output, &format!("unfiltered Go {task}"));
    }
    assert!(
        unfiltered
            .path()
            .join("apps/api/dist")
            .join(if cfg!(windows) { "api.exe" } else { "api" })
            .exists(),
        "unfiltered native build must produce the runnable binary"
    );

    let filtered = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(filtered.path());
    let build_args = [
        "run",
        "build",
        "--filter=example.com/api",
        "--log-order=grouped",
    ];
    let binary = filtered
        .path()
        .join("apps/api/dist")
        .join(if cfg!(windows) { "api.exe" } else { "api" });

    let output = run_turbo(filtered.path(), &build_args);
    assert_command_success(&output, "cold filtered Go build");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cache miss"),
        "expected cache miss: {stdout}"
    );
    assert!(binary.exists(), "native build must produce {binary:?}");

    let output = run_turbo(filtered.path(), &build_args);
    assert_command_success(&output, "warm filtered Go build");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "second build must hit cache: {stdout}"
    );

    fs::remove_dir_all(binary.parent().expect("binary output directory")).unwrap();
    let output = run_turbo(filtered.path(), &build_args);
    assert_command_success(&output, "restored filtered Go build");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "restoration must come from cache: {stdout}"
    );
    assert!(binary.exists(), "cache hit must restore the binary");

    let output = run_turbo(
        filtered.path(),
        &[
            "run",
            "dev",
            "--filter=example.com/api",
            "--",
            "passed-to-go",
        ],
    );
    assert_command_success(&output, "native Go dev with pass-through argument");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("passed-to-go"),
        "go run must receive pass-through arguments: {output:?}"
    );
}

#[test]
fn test_go_format_override_exclusion_and_failure_propagation() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    let library = tempdir.path().join("packages/lib/lib.go");
    fs::write(&library, "package lib\nfunc   Greet( ){ }\n").unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["run", "format", "--filter=example.com/lib"],
    );
    assert_command_success(&output, "filtered native Go format");
    assert_eq!(
        fs::read_to_string(&library).unwrap(),
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
    assert_command_success(&output, "authored Go build override");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("go version go"),
        "the authored command must execute: {output:?}"
    );
    assert!(
        !tempdir.path().join("apps/api/dist").exists(),
        "the native build must not shadow the authored command"
    );

    fs::write(
        tempdir.path().join("apps/api/turbo.json"),
        r#"{
  "extends": ["//"],
  "tasks": {
    "build": { "extends": false }
  }
}"#,
    )
    .unwrap();
    let tasks = package_task_names(tempdir.path(), "example.com/api");
    assert!(
        !tasks.iter().any(|task| task == "build"),
        "package task exclusion must remove the inherited command: {tasks:?}"
    );

    fs::write(
        tempdir.path().join("packages/lib/lib_test.go"),
        "package lib\n\nimport \"testing\"\n\nfunc TestFailure(t *testing.T) { \
         t.Fatal(\"intentional failure\") }\n",
    )
    .unwrap();
    let output = run_turbo(tempdir.path(), &["run", "test", "--filter=example.com/lib"]);
    assert!(
        !output.status.success(),
        "a failing Go test must fail the Turbo task"
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("intentional failure"),
        "Go failure output must propagate: {combined}"
    );
}

#[test]
fn test_go_facts_are_consistent_across_query_dry_run_and_summary() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    let executable = if cfg!(windows) { "api.exe" } else { "api" };
    let output_path = format!("dist/{executable}");
    let build_command = format!("go build -o {output_path} .");
    let task_directory = Path::new("apps").join("api").to_string_lossy().into_owned();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { api: package(name: \"example.com/api\") { name path directDependencies { \
             items { name } } tasks { items { name command directDependencies { items { fullName \
             } } } } } aggregate: package(name: \"go-workspace\") { name path tasks { items { \
             name command } } } }",
        ],
    );
    assert_command_success(&output, "Go package and task query");
    let query: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("query emits JSON");
    let api = &query["data"]["api"];
    assert_eq!(api["name"], "example.com/api");
    assert_eq!(api["path"], "apps/api");
    assert!(
        api["directDependencies"]["items"]
            .as_array()
            .is_some_and(|dependencies| dependencies
                .iter()
                .any(|dependency| dependency["name"] == "example.com/lib"))
    );
    let queried_build = api["tasks"]["items"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|task| task["name"] == "build"))
        .expect("queried Go build task");
    assert_eq!(queried_build["command"], build_command);
    assert!(
        queried_build["directDependencies"]["items"]
            .as_array()
            .is_some_and(|dependencies| dependencies
                .iter()
                .any(|dependency| dependency["fullName"] == "example.com/lib#build"))
    );

    let aggregate = &query["data"]["aggregate"];
    assert_eq!(aggregate["name"], "go-workspace");
    assert_eq!(aggregate["path"], "");
    assert!(
        aggregate["tasks"]["items"]
            .as_array()
            .is_some_and(|tasks| tasks.iter().any(|task| {
                task["name"] == "test"
                    && task["command"] == "go test ./apps/api/... ./packages/lib/..."
            }))
    );

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=example.com/api", "--dry-run=json"],
    );
    let dry_run = dry_run_task(&output, "example.com/api#build");
    assert_eq!(dry_run["package"], "example.com/api");
    assert_eq!(dry_run["directory"], task_directory);
    assert_eq!(dry_run["command"], build_command);
    assert!(
        dry_run["resolvedTaskDefinition"]["inputs"]
            .as_array()
            .is_some_and(|inputs| inputs.iter().any(|input| input == "../../go.work"))
    );
    assert!(
        dry_run["resolvedTaskDefinition"]["outputs"]
            .as_array()
            .is_some_and(|outputs| outputs.iter().any(|output| output == &output_path))
    );
    assert!(
        dry_run["hashOfExternalDependencies"]
            .as_str()
            .is_some_and(|hash| !hash.is_empty())
    );

    let output = run_turbo(tempdir.path(), &["run", "build", "--summarize"]);
    assert_command_success(&output, "summarized Go build");
    let summary_path = fs::read_dir(tempdir.path().join(".turbo/runs"))
        .expect("run summary directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .expect("Go run summary");
    let summary: serde_json::Value =
        serde_json::from_slice(&fs::read(summary_path).expect("read Go run summary"))
            .expect("parse Go run summary");
    let summarized_build = summary["tasks"]
        .as_array()
        .and_then(|tasks| {
            tasks
                .iter()
                .find(|task| task["taskId"] == "example.com/api#build")
        })
        .expect("summarized Go build task");
    assert_eq!(summarized_build["package"], "example.com/api");
    assert_eq!(summarized_build["directory"], task_directory);
    assert_eq!(summarized_build["command"], build_command);
    assert!(
        summarized_build["inputs"]
            .as_object()
            .is_some_and(|inputs| inputs.contains_key("main.go") && inputs.contains_key("go.mod"))
    );
    assert!(
        summarized_build["outputs"]
            .as_array()
            .is_some_and(|outputs| outputs.iter().any(|output| output == &output_path))
    );
    assert!(
        summarized_build["hashOfExternalDependencies"]
            .as_str()
            .is_some_and(|hash| !hash.is_empty())
    );
}

#[test]
fn test_mixed_repository_query_keeps_external_resolution_domains_separate() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_monorepo(tempdir.path());
    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { externalDependencies { items { name internalDependents { items { name } } } \
             } }",
        ],
    );
    assert_command_success(&output, "mixed external dependency query");
    let query: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("query emits JSON");
    let packages = query["data"]["externalDependencies"]["items"]
        .as_array()
        .expect("external dependencies");
    let dependents = |external_name: &str| {
        packages
            .iter()
            .find(|package| package["name"] == external_name)
            .and_then(|package| package["internalDependents"]["items"].as_array())
            .unwrap_or_else(|| panic!("{external_name} and its dependents"))
    };

    let go_dependents = dependents("go");
    for package in ["example.com/api", "example.com/lib", "go-workspace"] {
        assert!(
            go_dependents
                .iter()
                .any(|dependent| dependent["name"] == package),
            "{package} must stay in the Go resolution domain: {go_dependents:?}"
        );
    }
    assert!(
        !go_dependents
            .iter()
            .any(|dependent| dependent["name"] == "js-pkg")
    );

    let js_dependents = dependents("picocolors@1.1.1");
    assert_eq!(js_dependents.len(), 1);
    assert_eq!(js_dependents[0]["name"], "js-pkg");
}
