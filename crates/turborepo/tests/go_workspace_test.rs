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

fn normalize_rendered_diagnostic(diagnostic: &str) -> String {
    diagnostic
        .lines()
        .map(|line| {
            line.trim_start()
                .strip_prefix("| ")
                .unwrap_or_else(|| line.trim_start())
        })
        .collect::<Vec<_>>()
        .join(" ")
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
    // Library dependencies run uncached; check the executable's cache status.
    let expected = format!("example.com/api:build: {expected}");
    assert!(
        combined.contains(&expected),
        "{context} must report {expected:?}\noutput:\n{combined}"
    );
}

fn alternate_go_arch(host_arch: &str) -> &'static str {
    if host_arch == "arm64" {
        "amd64"
    } else {
        "arm64"
    }
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
fn publish_go_work(path: &Path, contents: &[u8]) {
    use std::io::Write;

    // Stage on the same filesystem so persist replaces the manifest atomically.
    let mut staged = tempfile::NamedTempFile::new_in(path.parent().unwrap()).unwrap();
    staged.write_all(contents).unwrap();
    staged.persist(path).unwrap();
}

#[cfg(unix)]
struct GoWatchGuard {
    child: Child,
    // Keep diagnostics/config outside the watched repository: logging must not
    // itself generate file events or become a task input.
    diagnostics: tempfile::TempDir,
}

#[cfg(unix)]
impl GoWatchGuard {
    fn spawn(dir: &Path) -> Self {
        let mut command = std::process::Command::new(assert_cmd::cargo::cargo_bin("turbo"));
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
            .env_remove("CI")
            .env_remove("GITHUB_ACTIONS")
            .current_dir(dir);
        Self::spawn_command(command)
    }

    fn spawn_command(mut command: std::process::Command) -> Self {
        use std::os::unix::process::CommandExt;

        let diagnostics = tempfile::tempdir().expect("create watch diagnostics directory");
        let stdout = fs::File::create(diagnostics.path().join("stdout.log")).unwrap();
        let stderr = fs::File::create(diagnostics.path().join("stderr.log")).unwrap();
        // Files avoid pipe backpressure and reader threads that can outlive the
        // child. The guard keeps both the logs and isolated config alive.
        let child = command
            .process_group(0)
            .env("TURBO_CONFIG_DIR_PATH", diagnostics.path().join("config"))
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .spawn()
            .expect("failed to spawn watch command");
        Self { child, diagnostics }
    }

    fn output(&self) -> String {
        use std::io::{Read, Seek, SeekFrom};

        let read = |name| -> std::io::Result<String> {
            // Keep a noisy child from overwhelming the test failure report.
            const MAX_BYTES: u64 = 64 * 1024;
            let mut file = fs::File::open(self.diagnostics.path().join(name))?;
            let skipped = file.metadata()?.len().saturating_sub(MAX_BYTES);
            file.seek(SeekFrom::Start(skipped))?;
            let mut bytes = Vec::new();
            file.take(MAX_BYTES).read_to_end(&mut bytes)?;
            let text = String::from_utf8_lossy(&bytes);
            Ok(if skipped > 0 {
                format!("[omitted {skipped} bytes]\n{text}")
            } else {
                text.into_owned()
            })
        };
        let log =
            |name| read(name).unwrap_or_else(|error| format!("failed to read {name}: {error}"));
        format!(
            "stdout:\n{}\nstderr:\n{}",
            log("stdout.log"),
            log("stderr.log")
        )
    }

    fn wait_for_path(&mut self, path: &Path, timeout: Duration) -> Result<(), String> {
        let started = std::time::Instant::now();
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    return Err(format!(
                        "watch exited with {status} while waiting for {path:?}\n{}",
                        self.output()
                    ));
                }
                Err(error) => {
                    return Err(format!("polling watch failed: {error}\n{}", self.output()));
                }
                Ok(None) => {}
            }
            if path.exists() {
                return Ok(());
            }
            if started.elapsed() >= timeout {
                return Err(format!(
                    "timed out after {timeout:?} waiting for {path:?}\n{}",
                    self.output()
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(unix)]
impl Drop for GoWatchGuard {
    fn drop(&mut self) {
        use nix::{
            sys::signal::{self, Signal},
            unistd::Pid,
        };

        let child = &mut self.child;
        // Avoid signaling a recycled process-group ID if a prior wait already
        // observed and reaped the child.
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) => {}
        }
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

#[cfg(unix)]
mod go_watch_harness_tests {
    use super::*;

    #[test]
    fn atomic_publication_never_exposes_partial_go_work() {
        use std::sync::{Barrier, mpsc};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("go.work");
        let old = format!(
            "go 1.22\nuse ./apps/api\n{}",
            "// old workspace\n".repeat(16384)
        );
        let new = format!(
            "go 1.22\nuse ./apps/worker\n{}",
            "// new workspace\n".repeat(16384)
        );
        fs::write(&path, &old).unwrap();
        let ready = Barrier::new(2);
        std::thread::scope(|scope| {
            // Dropping the sender also stops the reader if publication panics.
            let (done, completed) = mpsc::channel::<()>();
            let (first_read, observed_first_read) = mpsc::channel::<()>();
            let reader = scope.spawn({
                let (path, old, new, ready) = (&path, &old, &new, &ready);
                move || {
                    ready.wait();
                    let mut first = true;
                    loop {
                        let observed = fs::read(path).unwrap();
                        assert!(
                            observed == old.as_bytes() || observed == new.as_bytes(),
                            "reader observed a partial go.work ({} bytes)",
                            observed.len()
                        );
                        if first {
                            first_read.send(()).unwrap();
                            first = false;
                        }
                        if !matches!(completed.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                            break;
                        }
                    }
                }
            });
            ready.wait();
            observed_first_read.recv().unwrap();
            for _ in 0..100 {
                publish_go_work(&path, new.as_bytes());
                publish_go_work(&path, old.as_bytes());
            }
            drop(done);
            reader.join().unwrap();
        });
    }

    #[test]
    fn wait_reports_exit_even_if_output_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output");
        fs::write(&path, "not proof of a live watcher").unwrap();
        let mut command = std::process::Command::new("sh");
        command.args([
            "-c",
            "printf 'watch stdout'; printf 'watch stderr' >&2; exit 7",
        ]);
        let mut watch = GoWatchGuard::spawn_command(command);
        // Synchronize on exit rather than racing the child's final instructions.
        watch.child.wait().unwrap();
        let error = watch
            .wait_for_path(&path, Duration::from_secs(30))
            .unwrap_err();
        assert!(error.contains("exit status: 7"), "{error}");
        assert!(error.contains("watch stdout"), "{error}");
        assert!(error.contains("watch stderr"), "{error}");
        assert!(error.contains(path.to_str().unwrap()), "{error}");
    }

    #[test]
    fn noisy_child_does_not_block_and_timeout_includes_log_tails() {
        let dir = tempfile::tempdir().unwrap();
        let ready = dir.path().join("ready");
        let mut command = std::process::Command::new("sh");
        command
            .args([
                "-c",
                concat!(
                    "i=0; while [ $i -lt 8192 ]; do ",
                    "printf 'verbose watch output\\n'; printf 'verbose watch error\\n' >&2; ",
                    "i=$((i+1)); done; ",
                    "printf '\\377stdout tail\\n'; printf 'stderr tail\\n' >&2; ",
                    "touch \"$1\"; exec sleep 60"
                ),
                "watch-test",
            ])
            .arg(&ready);
        let mut watch = GoWatchGuard::spawn_command(command);
        watch
            .wait_for_path(&ready, Duration::from_secs(30))
            .unwrap();
        let missing = dir.path().join("missing");
        let error = watch.wait_for_path(&missing, Duration::ZERO).unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(error.contains("stdout tail"), "{error}");
        assert!(error.contains("stderr tail"), "{error}");
        assert!(error.contains("[omitted "), "{error}");
        assert!(error.len() < 132 * 1024, "diagnostics must be bounded");
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
fn test_go_build_dependencies_and_hash_do_not_depend_on_entrypoint() {
    assert_go_build_dependencies_and_hash_do_not_depend_on_entrypoint(false);
}

#[test]
fn test_go_build_dependencies_and_hash_with_task_filtering() {
    assert_go_build_dependencies_and_hash_do_not_depend_on_entrypoint(true);
}

fn assert_go_build_dependencies_and_hash_do_not_depend_on_entrypoint(filter_using_tasks: bool) {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    let root = tempdir.path();
    setup_go_pure_workspace(root);
    for module in ["apps/api", "packages/lib"] {
        assert!(!root.join(module).join("package.json").exists());
    }
    let config_path = root.join("turbo.json");
    let mut config: serde_json::Value =
        serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["futureFlags"]["filterUsingTasks"] = serde_json::json!(filter_using_tasks);
    config["tasks"]["typecheck"] = serde_json::json!({ "dependsOn": ["^build"] });
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

    let indirect = run_turbo(root, &["run", "typecheck", "--dry-run=json"]);
    let indirect_api = dry_run_task(&indirect, "example.com/api#build");
    let expected_dependencies = serde_json::json!(["example.com/lib#build"]);
    assert_eq!(indirect_api["dependencies"], expected_dependencies);

    for args in [
        vec!["run", "build", "--dry-run=json"],
        vec!["run", "build", "--only", "--dry-run=json"],
        vec!["run", "build", "typecheck", "--dry-run=json"],
        vec!["run", "example.com/api#build", "--dry-run=json"],
        vec!["run", "build", "--filter=example.com/api", "--dry-run=json"],
    ] {
        let direct = run_turbo(root, &args);
        let direct_api = dry_run_task(&direct, "example.com/api#build");
        assert_eq!(
            direct_api["dependencies"], expected_dependencies,
            "{args:?}"
        );
        assert_eq!(
            direct_api["resolvedTaskDefinition"], indirect_api["resolvedTaskDefinition"],
            "{args:?}"
        );
        assert_eq!(direct_api["hash"], indirect_api["hash"], "{args:?}");
    }
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
        stdout.contains("js-pkg:build: cache hit")
            && stdout.contains("example.com/api:build: cache hit"),
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
    // Diagnostic renderers can wrap this sentence at different words and add a
    // continuation gutter depending on paths and platform terminal widths.
    let normalized_stderr = normalize_rendered_diagnostic(&stderr);
    assert!(
        normalized_stderr.contains("`go work edit -json` failed")
            && normalized_stderr.contains(
                "Repair the repository-root go.work with `go work edit` and `go work use`."
            ),
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
    let stderr = String::from_utf8_lossy(&output.stderr).replace('\\', "/");
    assert!(
        stderr.contains("Failed to add workspace \"js-pkg\"")
            && stderr.contains("apps/api/go.mod")
            && stderr.contains("packages/js-pkg/package.json")
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
fn test_go_cache_alternate_arch_is_supported_on_ci_platforms() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    let supported = run_go(tempdir.path(), &["tool", "dist", "list"]);
    assert_command_success(&supported, "list supported Go targets");
    let supported = String::from_utf8_lossy(&supported.stdout);

    for goos in ["darwin", "linux", "windows"] {
        for host_arch in ["amd64", "arm64"] {
            let target_arch = alternate_go_arch(host_arch);
            assert_ne!(
                target_arch, host_arch,
                "cache invalidation target must differ from the host architecture"
            );
            let target = format!("{goos}/{target_arch}");
            assert!(
                supported.lines().any(|candidate| candidate == target),
                "{target} must remain a supported Go cross-compilation target"
            );
        }
    }
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
    assert_go_build_cache_result(root, &[], "cache hit", "unchanged Go build");

    fs::write(
        root.join("tools/independent/independent.go"),
        "package independent\n\nconst Value = \"still-independent\"\n",
    )
    .unwrap();
    assert_go_build_cache_result(root, &[], "cache hit", "unrelated module source change");

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
    let target_arch = alternate_go_arch(String::from_utf8_lossy(&go_arch.stdout).trim());
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
    let mut watch = GoWatchGuard::spawn(tempdir.path());
    let api_binary = tempdir.path().join("apps/api/dist/api");
    watch
        .wait_for_path(&api_binary, Duration::from_secs(30))
        .unwrap_or_else(|error| panic!("initial Go watch build failed: {error}"));

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
    // This test exercises rediscovery of a valid workspace, not recovery from
    // a partially written manifest. fs::write truncates go.work before writing;
    // the watcher can observe an empty workspace and exit before the write ends.
    publish_go_work(
        &tempdir.path().join("go.work"),
        b"go 1.22\n\nuse (\n\t./apps/api\n\t./apps/worker\n\t./packages/lib\n)\n",
    );
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
    watch
        .wait_for_path(&worker_binary, Duration::from_secs(60))
        .unwrap_or_else(|error| panic!("Go watch workspace rediscovery failed: {error}"));
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
        stdout.contains("example.com/api:build: cache hit"),
        "second build must hit cache: {stdout}"
    );

    fs::remove_dir_all(binary.parent().expect("binary output directory")).unwrap();
    let output = run_turbo(filtered.path(), &build_args);
    assert_command_success(&output, "restored filtered Go build");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("example.com/api:build: cache hit"),
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

#[test]
fn test_go_regression_profile_outputs_are_not_log_only_cache_hits() {
    if !go_available() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let outputs = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(root.path());
    let profile = outputs.path().join("coverage.out");
    let flag = format!("-coverprofile={}", profile.display());
    let cache_path = cache.path().to_str().unwrap();
    // Cover both a module and the workspace aggregate, which derive their IO
    // separately. The report deliberately lives outside default input globs.
    for filter in ["--filter=example.com/api", "--filter=go-workspace"] {
        let args = ["run", "test", filter, "--", &flag];
        for _ in 0..2 {
            let output = run_turbo_with_env(root.path(), &args, &[("GOCACHE", cache_path)]);
            assert_command_success(&output, "test with coverage output");
            assert!(
                profile.exists(),
                "every run must produce the uncaptured report"
            );
            fs::remove_file(&profile).unwrap();
        }
    }
}

#[test]
fn test_go_regression_strict_execution_preserves_goenv_and_godebug() {
    if !go_available() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let settings = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(root.path());
    let goenv = settings.path().join("goenv");
    fs::write(&goenv, "GOFLAGS=-tags=goenvregression\n").unwrap();
    fs::write(
        root.path().join("apps/api/env_test.go"),
        r#"package main
import ("os"; "testing")
func TestEnvironment(t *testing.T) {
    if os.Getenv("GODEBUG") != "panicnil=1" { t.Fatal("GODEBUG was dropped") }
}
"#,
    )
    .unwrap();
    fs::write(
        root.path().join("apps/api/missing_env_test.go"),
        r#"//go:build !goenvregression

package main
import "testing"
func TestMissingGoenv(t *testing.T) { t.Fatal("GOENV settings were dropped") }
"#,
    )
    .unwrap();
    let output = run_turbo_with_env(
        root.path(),
        &["run", "test", "--filter=example.com/api"],
        &[
            ("GOENV", goenv.to_str().unwrap()),
            ("GODEBUG", "panicnil=1"),
        ],
    );
    assert_command_success(&output, "strict Go execution with custom GOENV and GODEBUG");
}

#[test]
fn test_go_regression_native_output_and_test_argument_placement() {
    if !go_available() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let outputs = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(root.path());
    let binary = outputs.path().join(if cfg!(windows) {
        "custom.exe"
    } else {
        "custom"
    });
    let cache_path = cache.path().to_str().unwrap();
    let output = run_turbo_with_env(
        root.path(),
        &[
            "run",
            "build",
            "--filter=example.com/api",
            "--",
            "-o",
            binary.to_str().unwrap(),
        ],
        &[("GOCACHE", cache_path)],
    );
    assert_command_success(&output, "Go build with custom output");
    assert!(
        binary.exists(),
        "the built-in -o must not override user arguments"
    );

    let subpackage = root.path().join("apps/api/subpackage");
    fs::create_dir_all(&subpackage).unwrap();
    fs::write(
        subpackage.join("args_test.go"),
        r#"package subpackage
import ("flag"; "testing")
var custom = flag.String("custom", "", "custom test argument")
func TestCustom(t *testing.T) {
    if *custom != "expected" { t.Fatalf("argument not passed: %q", *custom) }
}
"#,
    )
    .unwrap();
    for filter in ["--filter=example.com/api", "--filter=go-workspace"] {
        let output = run_turbo_with_env(
            root.path(),
            &["run", "test", filter, "--", "-args", "-custom=expected"],
            &[("GOCACHE", cache_path)],
        );
        assert_command_success(&output, "Go test with test-binary arguments");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("example.com/api/subpackage"),
            "package patterns must precede -args so subpackages are tested"
        );
    }
}

#[test]
fn test_go_regression_goflags_and_explicit_outputs_do_not_hide_untracked_inputs() {
    if !go_available() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(root.path());
    let settings = tempfile::tempdir().unwrap();
    let goenv = settings.path().join("goenv");
    fs::write(&goenv, "GOFLAGS=-buildmode=c-shared\n").unwrap();
    let output = run_turbo_with_env(
        root.path(),
        &["run", "build", "--filter=example.com/api", "--dry-run=json"],
        &[("GOENV", goenv.to_str().unwrap())],
    );
    let build = dry_run_task(&output, "example.com/api#build");
    assert_eq!(build["resolvedTaskDefinition"]["cache"], false);

    fs::write(
        root.path().join("apps/api/turbo.json"),
        r#"{
        "extends": ["//"], "tasks": { "build": { "outputs": ["dist/**"] } }
    }"#,
    )
    .unwrap();
    let output = run_turbo(
        root.path(),
        &[
            "run",
            "build",
            "--filter=example.com/api",
            "--dry-run=json",
            "--",
            "-overlay=elsewhere.json",
        ],
    );
    let build = dry_run_task(&output, "example.com/api#build");
    assert_eq!(
        build["resolvedTaskDefinition"]["cache"], false,
        "explicit outputs do not describe an overlay's untracked source inputs"
    );
}
