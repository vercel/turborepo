//! End-to-end tests for experimental Go workspace support.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path, sync::OnceLock};
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
    "GOPATH",
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

fn shared_go_caches() -> &'static Path {
    static CACHES: OnceLock<std::path::PathBuf> = OnceLock::new();
    CACHES
        .get_or_init(|| common::integration_toolchain_cache_dir("go"))
        .as_path()
}

/// Opt in to cold Go compilation/module caches when the cache itself is under
/// test.
fn cold_go_caches() -> tempfile::TempDir {
    tempfile::tempdir().expect("failed to create cold Go cache tempdir")
}

fn run_turbo(dir: &Path, args: &[&str]) -> std::process::Output {
    run_turbo_with_env(dir, args, &[])
}

fn run_turbo_with_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> std::process::Output {
    run_turbo_with_env_and_go_caches(dir, args, env, shared_go_caches())
}

fn run_turbo_with_env_and_go_caches(
    dir: &Path,
    args: &[&str],
    env: &[(&str, &str)],
    caches: &Path,
) -> std::process::Output {
    let config_dir = tempfile::tempdir().expect("failed to create config tempdir");
    let mut command = common::turbo_command(dir);
    for name in AMBIENT_GO_ENV {
        command.env_remove(name);
    }
    command
        .env("GOCACHE", caches.join("go-build"))
        .env("GOMODCACHE", caches.join("go-mod"))
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
    run_go_with_caches(dir, args, shared_go_caches())
}

fn run_go_with_caches(dir: &Path, args: &[&str], caches: &Path) -> std::process::Output {
    let mut command = std::process::Command::new("go");
    for name in AMBIENT_GO_ENV {
        command.env_remove(name);
    }
    command
        .args(args)
        .env("GOCACHE", caches.join("go-build"))
        .env("GOMODCACHE", caches.join("go-mod"))
        .env("GOENV", "off")
        .env("GOTOOLCHAIN", "local")
        .current_dir(dir)
        .output()
        .expect("failed to execute go")
}

#[test]
fn test_go_compilation_caches_are_shared_unless_cold_requested() {
    if !go_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let read_caches = |caches: &Path| {
        let output = run_go_with_caches(
            dir.path(),
            &["env", "GOCACHE", "GOMODCACHE", "GOENV", "GOTOOLCHAIN"],
            caches,
        );
        assert_command_success(&output, "read Go cache environment");
        String::from_utf8(output.stdout).unwrap()
    };
    let shared = read_caches(shared_go_caches());
    assert_eq!(shared, read_caches(shared_go_caches()));
    let cold = cold_go_caches();
    let cold_env = read_caches(cold.path());
    assert_ne!(shared, cold_env);
    let expected = |caches: &Path| {
        format!(
            "{}\n{}\n\nlocal\n",
            caches.join("go-build").display(),
            caches.join("go-mod").display()
        )
    };
    assert_eq!(shared, expected(shared_go_caches()));
    assert_eq!(cold_env, expected(cold.path()));
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
        &["run", "build", "--filter=api", "--log-order=grouped"],
        environment,
    );
    assert_command_success(&output, context);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // Library dependencies run uncached; check the executable's cache status.
    let expected = format!("api:build: {expected}");
    assert!(
        combined.contains(&expected),
        "{context} must report {expected:?}\noutput:\n{combined}"
    );
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

// Keep assembled-binary checks of final hash equality and the
// filterUsingTasks path; the in-process engine matrix covers other selections.
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
    let indirect_api = dry_run_task(&indirect, "api#build");
    let expected_dependencies = serde_json::json!(["lib#build"]);
    assert_eq!(indirect_api["dependencies"], expected_dependencies);

    let direct = run_turbo(root, &["run", "build", "--only", "--dry-run=json"]);
    let direct_api = dry_run_task(&direct, "api#build");
    assert_eq!(direct_api["dependencies"], expected_dependencies);
    assert_eq!(
        direct_api["resolvedTaskDefinition"],
        indirect_api["resolvedTaskDefinition"]
    );
    assert_eq!(direct_api["hash"], indirect_api["hash"]);
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
            .join("apps/api")
            .join(if cfg!(windows) { "api.exe" } else { "api" })
            .exists(),
        "the Go module must produce its native executable"
    );

    let output = run_turbo(tempdir.path(), &args);
    assert_command_success(&output, "warm mixed JavaScript and Go build");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("js-pkg:build: cache hit") && stdout.contains("api:build: cache hit"),
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
        .find(|package| package["name"] == "api")
        .expect("api package");
    let dependencies: Vec<&str> = api["directDependencies"]["items"]
        .as_array()
        .expect("dependencies")
        .iter()
        .filter_map(|dependency| dependency["name"].as_str())
        .collect();
    assert_eq!(dependencies, ["lib"]);
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

    let output = run_turbo(tempdir.path(), &["prune", "api", "--docker"]);
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
        // Prune accepts the short package name, but must not rewrite Go paths.
        for manifest in ["apps/api/go.mod", "packages/lib/go.mod"] {
            assert_eq!(
                fs::read(root.join(manifest)).unwrap(),
                fs::read(tempdir.path().join(manifest)).unwrap(),
                "pruned {manifest} must preserve module, require, and replace paths"
            );
        }
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
        let output = run_turbo(&full, &["run", task, "--filter=api", "--log-order=grouped"]);
        assert_command_success(&output, &format!("pruned native Go {task} task"));
    }
    assert!(
        full.join("apps/api")
            .join(if cfg!(windows) { "api.exe" } else { "api" })
            .exists(),
        "the pruned native build must produce its executable"
    );
}

#[test]
fn test_go_package_names_preserve_major_versions_without_aliases() {
    if !go_available() {
        return;
    }

    for suffix in ["", "/v2", "/v10"] {
        let tempdir = tempfile::tempdir().unwrap();
        let root = tempdir.path();
        setup_go_pure_workspace(root);
        let name = format!("api{suffix}");
        let module_path = format!("example.com/{name}");
        fs::write(
            root.join("apps/api/go.mod"),
            format!(
                "module {module_path}\n\ngo 1.22\n\nrequire example.com/lib v0.0.0\n\nreplace \
                 example.com/lib => ../../packages/lib\n"
            ),
        )
        .unwrap();

        let names = package_names(root);
        assert!(names.contains(&name), "names: {names:?}");
        assert!(names.contains(&"lib".to_string()), "names: {names:?}");
        assert!(
            !names.contains(&module_path),
            "no full-path alias: {names:?}"
        );
        if !suffix.is_empty() {
            assert!(
                !names.contains(&"api".to_string()),
                "no unversioned alias: {names:?}"
            );
            assert!(
                !names.contains(&suffix.trim_start_matches('/').to_string()),
                "the major version alone is not a package name: {names:?}"
            );
        }

        let task_id = format!("{name}#build");
        let filtered = run_turbo(
            root,
            &[
                "run",
                "build",
                &format!("--filter={name}"),
                "--dry-run=json",
            ],
        );
        let task = dry_run_task(&filtered, &task_id);
        assert_eq!(task["package"], name);
        assert_eq!(task["dependencies"], serde_json::json!(["lib#build"]));
        let explicit = run_turbo(root, &["run", &task_id, "--dry-run=json"]);
        assert_eq!(task["hash"], dry_run_task(&explicit, &task_id)["hash"]);

        // Full module paths are Go metadata, not alternate Turborepo selectors.
        let mut aliases = vec![module_path.clone()];
        if !suffix.is_empty() {
            aliases.push("api".to_string());
        }
        for alias in aliases {
            for args in [
                vec![
                    "run".to_string(),
                    "build".to_string(),
                    format!("--filter={alias}"),
                    "--dry-run=json".to_string(),
                ],
                vec![
                    "run".to_string(),
                    format!("{alias}#build"),
                    "--dry-run=json".to_string(),
                ],
            ] {
                let args = args.iter().map(String::as_str).collect::<Vec<_>>();
                let output = run_turbo(root, &args);
                assert!(
                    !output.status.success(),
                    "{alias} must not select {name}: {}",
                    common::combined_output(&output)
                );
            }
        }

        let output = run_go(&root.join("apps/api"), &["list", "-json", "."]);
        assert_command_success(&output, "Go metadata after short-name task selection");
        let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(metadata["Module"]["Path"], module_path);
        assert!(
            metadata["Imports"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("example.com/lib")),
            "Go imports must retain full module paths: {metadata}"
        );
    }
}

#[test]
fn test_go_packages_with_same_short_name_report_both_manifests() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    let root = tempdir.path();
    setup_go_pure_workspace(root);
    let other = root.join("tools/other-api");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("go.mod"), "module example.net/api\n\ngo 1.22\n").unwrap();
    fs::write(other.join("api.go"), "package api\n").unwrap();
    fs::write(
        root.join("go.work"),
        "go 1.22\n\nuse (\n\t./apps/api\n\t./packages/lib\n\t./tools/other-api\n)\n",
    )
    .unwrap();

    // Filters cannot hide a collision in either listing or lazy run planning.
    for args in [
        vec!["ls", "--filter=lib"],
        vec!["run", "build", "--filter=lib", "--dry-run=json"],
    ] {
        let output = run_turbo(root, &args);
        assert!(
            !output.status.success(),
            "duplicate names must fail: {args:?}"
        );
        let stderr = String::from_utf8_lossy(&output.stderr).replace('\\', "/");
        assert!(
            stderr.contains("Failed to add workspace \"api\"")
                && stderr.contains("apps/api/go.mod")
                && stderr.contains("tools/other-api/go.mod"),
            "short-name collision must identify both Go manifests: {stderr}"
        );
    }
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

    let output = run_turbo(tempdir.path(), &["ls", "--filter=api"]);
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

    // `ls` is an unfiltered, repository-wide query: lazy native discovery
    // may load every contributor for it. The Go scope inventory itself needs
    // no `go` binary, but the loaded owner does, so a missing `go` fails the
    // listing with the ordinary missing-toolchain diagnostic.
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

    // Selecting a Go task narrows the query to the Go scope, which loads the
    // same owner; the diagnostic keeps its requirement and remediation.
    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--filter=api", "--dry-run=json"],
        &[("PATH", "")],
    );
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
    // The scope inventory parses go.work in-process, so it detects the
    // malformed directive before any `go` subprocess: with an empty PATH the
    // diagnostic is identical, which proves no toolchain ran.
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    fs::write(
        tempdir.path().join("go.work"),
        "go 1.22\n\nunsupported ./apps/api\n",
    )
    .unwrap();

    let output = run_turbo_with_env(tempdir.path(), &["ls"], &[("PATH", "")]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    // The inventory parser diagnoses the unknown directive itself — the file,
    // the directive, the directives go.work supports, and the repair —
    // without pretending any `go` command ran and without falling back to
    // JavaScript discovery.
    let normalized_stderr = normalize_rendered_diagnostic(&stderr);
    assert!(
        normalized_stderr.contains("go.work at")
            && normalized_stderr.contains("contains an unknown `unsupported` directive")
            && normalized_stderr.contains(
                "The go command supports only `go`, `toolchain`, `use`, `replace`, and `godebug` \
                 directives in go.work."
            )
            && normalized_stderr.contains(
                "Repair the repository-root go.work with `go work edit` and `go work use`."
            ),
        "invalid workspace must have focused remediation: {stderr}"
    );
    assert!(!stderr.contains("package manager"), "{stderr}");
    assert!(!stderr.contains("failed to parse"), "{stderr}");
    assert!(!stderr.contains("go work edit -json"), "{stderr}");
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
        "module example.com/js-pkg\n\ngo 1.22\n\nrequire example.com/lib v0.0.0\n\nreplace \
         example.com/lib => ../../packages/lib\n",
    )
    .unwrap();

    // A JS-only selection must not let lazy Go discovery hide a collision.
    let unrelated = tempdir.path().join("packages/unrelated");
    fs::create_dir_all(&unrelated).unwrap();
    fs::write(
        unrelated.join("package.json"),
        r#"{"name":"unrelated","scripts":{"build":"echo unrelated"}}"#,
    )
    .unwrap();

    // JavaScript-only selection must not hide a collision in listing or run.
    for args in [
        vec!["ls", "--filter=unrelated"],
        vec!["run", "build", "--filter=unrelated", "--dry-run=json"],
    ] {
        let output = run_turbo(tempdir.path(), &args);
        assert!(
            !output.status.success(),
            "cross-language collision must fail"
        );
        let stderr = String::from_utf8_lossy(&output.stderr).replace('\\', "/");
        assert!(
            stderr.contains("Failed to add workspace \"js-pkg\"")
                && stderr.contains("apps/api/go.mod")
                && stderr.contains("packages/js-pkg/package.json")
                && stderr.contains("Rename one package or module"),
            "cross-language collision must remain actionable under narrow selection: {stderr}"
        );
    }
}

#[test]
fn test_versioned_go_name_does_not_collide_with_unversioned_javascript_name() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    let root = tempdir.path();
    setup_go_monorepo(root);
    fs::write(
        root.join("apps/api/go.mod"),
        "module example.com/js-pkg/v2\n\ngo 1.22\n\nrequire example.com/lib v0.0.0\n\nreplace \
         example.com/lib => ../../packages/lib\n",
    )
    .unwrap();

    let names = package_names(root);
    assert!(names.contains(&"js-pkg".to_string()), "names: {names:?}");
    assert!(names.contains(&"js-pkg/v2".to_string()), "names: {names:?}");
    for (name, directory) in [("js-pkg", "packages/js-pkg"), ("js-pkg/v2", "apps/api")] {
        let output = run_turbo(
            root,
            &[
                "run",
                "build",
                &format!("--filter={name}"),
                "--dry-run=json",
            ],
        );
        let task = dry_run_task(&output, &format!("{name}#build"));
        assert_eq!(task["package"], name);
        assert_eq!(
            Path::new(task["directory"].as_str().unwrap()),
            Path::new(directory)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let other = if name == "js-pkg" {
            "js-pkg/v2"
        } else {
            "js-pkg"
        };
        assert!(
            json["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .all(|task| task["package"] != other),
            "filtering {name} must not select {other}: {json}"
        );
    }
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

    let combined = run_turbo(
        tempdir.path(),
        &["run", "test", "lint", "format", "--dry-run=json"],
    );
    assert_command_success(&combined, "combined Go verification dry run");
    for (task_name, command) in [
        ("test", "go test ./..."),
        ("lint", "go vet ./..."),
        ("format", "go fmt ./..."),
    ] {
        let output = run_turbo(tempdir.path(), &["run", task_name, "--dry-run=json"]);
        assert_command_success(&output, "unfiltered Go verification dry run");
        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("dry run emits JSON");
        assert_eq!(json["tasks"].as_array().map(Vec::len), Some(2));
        for (package, directory) in [("api", "apps/api"), ("lib", "packages/lib")] {
            let task_id = format!("{package}#{task_name}");
            let task = dry_run_task(&output, &task_id);
            assert_eq!(task["command"], command);
            assert_eq!(
                Path::new(task["directory"].as_str().expect("task directory")),
                Path::new(directory)
            );
            assert_eq!(
                task["resolvedTaskDefinition"]["cache"],
                task_name != "format"
            );
            assert!(task["hash"].as_str().is_some_and(|hash| !hash.is_empty()));
            assert_eq!(task["hash"], dry_run_task(&combined, &task_id)["hash"]);

            for filter in [
                format!("--filter={package}"),
                format!("--filter=./{directory}"),
            ] {
                let filtered = run_turbo(
                    tempdir.path(),
                    &["run", task_name, &filter, "--dry-run=json"],
                );
                let filtered_task = dry_run_task(&filtered, &task_id);
                assert_eq!(
                    task["hash"], filtered_task["hash"],
                    "{task_id} with {filter}"
                );
                assert_eq!(task["command"], filtered_task["command"]);
                assert_eq!(task["dependencies"], filtered_task["dependencies"]);
            }
        }
    }
    let workspace_tasks = package_task_names(tempdir.path(), "go-workspace");
    assert!(
        !workspace_tasks
            .iter()
            .any(|task| matches!(task.as_str(), "test" | "lint" | "format")),
        "verification must only be registered on modules: {workspace_tasks:?}"
    );

    let output = run_turbo(tempdir.path(), &["run", "build", "--dry-run=json"]);
    let build = dry_run_task(&output, "api#build");
    let executable = if cfg!(windows) { "api.exe" } else { "api" };
    let output_path = executable;
    assert_eq!(build["command"], "go build .");
    assert_eq!(build["resolvedTaskDefinition"]["cache"], true);
    assert!(
        build["resolvedTaskDefinition"]["outputs"]
            .as_array()
            .is_some_and(|outputs| outputs.iter().any(|output| output == output_path))
    );

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=lib", "--dry-run=json"],
    );
    let build = dry_run_task(&output, "lib#build");
    assert_eq!(build["command"], "go build ./...");
    assert_eq!(build["resolvedTaskDefinition"]["cache"], false);

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "dev",
            "--filter=api",
            "--dry-run=json",
            "--",
            "--port",
            "3000",
        ],
    );
    let dev = dry_run_task(&output, "api#dev");
    assert_eq!(dev["command"], "go run .");

    let tasks = package_task_names(tempdir.path(), "api");
    assert!(tasks.iter().any(|task| task == "dev"), "tasks: {tasks:?}");
    assert!(tasks.iter().any(|task| task == "lint"), "tasks: {tasks:?}");
    assert!(!tasks.iter().any(|task| task == "run"), "tasks: {tasks:?}");
    assert!(!tasks.iter().any(|task| task == "vet"), "tasks: {tasks:?}");

    let output = run_turbo(
        tempdir.path(),
        &["run", "run", "--filter=api", "--dry-run=json"],
    );
    assert!(!output.status.success(), "removed run task must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Could not find task `run` in project"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn test_go_verification_reuses_cache_across_filtered_and_unfiltered_runs() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    for task in ["test", "lint"] {
        for (filter, expected) in [
            (Some("--filter=./packages/lib"), vec![("lib", "cache miss")]),
            (None, vec![("lib", "cache hit"), ("api", "cache miss")]),
            (Some("--filter=./apps/api"), vec![("api", "cache hit")]),
        ] {
            let mut args = vec!["run", task, "--log-order=grouped"];
            args.extend(filter);
            let output = run_turbo(tempdir.path(), &args);
            assert_command_success(&output, "Go verification cache reuse");
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            for (package, status) in expected {
                let expected = format!("{package}:{task}: {status}");
                assert!(
                    combined.contains(&expected),
                    "expected {expected:?}: {combined}"
                );
            }
        }
    }
}

#[test]
fn test_go_verification_hash_changes_only_for_module_and_dependents() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_e2e_workspace(tempdir.path());
    let args = ["run", "test", "lint", "--dry-run=json"];
    let before = run_turbo(tempdir.path(), &args);
    assert_command_success(&before, "original Go verification hashes");

    fs::write(
        tempdir.path().join("packages/lib/lib.go"),
        "package lib\n\nfunc Greet() string { return \"changed\" }\n",
    )
    .unwrap();
    let after = run_turbo(tempdir.path(), &args);
    assert_command_success(&after, "changed Go verification hashes");
    for task in ["test", "lint"] {
        for package in ["lib", "api"] {
            let task_id = format!("{package}#{task}");
            assert_ne!(
                dry_run_task(&before, &task_id)["hash"],
                dry_run_task(&after, &task_id)["hash"],
                "{task_id} must track changed module sources"
            );
        }
        let task_id = format!("independent#{task}");
        assert_eq!(
            dry_run_task(&before, &task_id)["hash"],
            dry_run_task(&after, &task_id)["hash"],
            "{task_id} must not track unrelated modules"
        );
    }
}

#[test]
fn test_go_hash_ignores_checkout_path_and_root_siblings() {
    if !go_available() {
        return;
    }
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    setup_go_e2e_workspace(first.path());
    setup_go_e2e_workspace(second.path());

    let original = task_hash(first.path(), "api", "build");
    fs::write(first.path().join("unrelated.txt"), "unrelated\n").unwrap();
    assert_eq!(original, task_hash(first.path(), "api", "build"));
    assert_eq!(original, task_hash(second.path(), "api", "build"));
}

#[test]
fn test_go_cache_invalidates_on_dependency_source_and_build_environment() {
    if !go_available() {
        return;
    }

    // Per-input invalidation (module/dependency/replacement sources, manifests,
    // checksums, disconnected modules, and the Go toolchain/environment
    // fingerprint) is covered by crate contracts in turborepo-repository's
    // go.rs. This smoke proves the assembled binary restores and invalidates
    // real Go builds through one file input and one environment input.
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_e2e_workspace(tempdir.path());
    let root = tempdir.path();
    let assert_cache_result = |environment: &[(&str, &str)], expected, context| {
        assert_go_build_cache_result(root, environment, expected, context);
    };

    assert_cache_result(&[], "cache miss", "cold Go build");
    assert_cache_result(&[], "cache hit", "unchanged Go build");

    fs::write(
        root.join("tools/independent/independent.go"),
        "package independent\n\nconst Value = \"still-independent\"\n",
    )
    .unwrap();
    assert_cache_result(&[], "cache hit", "unrelated module source change");

    fs::write(
        root.join("packages/lib/lib.go"),
        "package lib\n\nfunc Value() string { return \"dependency-changed\" }\n",
    )
    .unwrap();
    assert_cache_result(&[], "cache miss", "internal dependency change");

    assert_cache_result(
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
    let package = "api";
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
fn test_affected_colocated_packages_include_both_dependency_chains() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    let dir = tempdir.path();
    setup_go_monorepo(dir);
    fs::write(
        dir.join("packages/lib/package.json"),
        r#"{"name":"@repo/lib","scripts":{"build":"echo javascript build"}}"#,
    )
    .unwrap();
    let consumer_path = dir.join("packages/js-pkg/package.json");
    let mut consumer: serde_json::Value =
        serde_json::from_slice(&fs::read(&consumer_path).unwrap()).unwrap();
    consumer["dependencies"] = serde_json::json!({"@repo/lib": "*"});
    fs::write(consumer_path, serde_json::to_vec(&consumer).unwrap()).unwrap();
    fs::create_dir(dir.join("packages/unrelated")).unwrap();
    fs::write(
        dir.join("packages/unrelated/package.json"),
        r#"{"name":"unrelated","scripts":{"build":"echo unrelated"}}"#,
    )
    .unwrap();
    // Exercise package-based affectedness, not task-input matching, which can
    // already match multiple tasks in one directory.
    fs::write(
        dir.join("turbo.json"),
        r#"{
          "futureFlags": {
            "experimentalGoWorkspaces": true,
            "experimentalTaskCommand": true,
            "affectedUsingTaskInputs": false,
            "filterUsingTasks": false,
            "watchUsingTaskInputs": false
          },
          "tasks": {"build": {"dependsOn": ["^build"]}}
        }"#,
    )
    .unwrap();
    common::git(dir, &["add", "."]);
    common::git(
        dir,
        &[
            "commit",
            "-m",
            "add colocated package and consumer",
            "--quiet",
        ],
    );

    let source = dir.join("packages/lib/lib.go");
    let contents = fs::read_to_string(&source).unwrap();
    fs::write(source, format!("{contents}\n// Source changed.\n")).unwrap();
    let output = run_turbo_with_env(
        dir,
        &["run", "build", "--affected", "--dry=json"],
        &[("TURBO_SCM_BASE", "HEAD")],
    );
    assert_command_success(&output, "co-located package-based affectedness");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let actual = json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["taskId"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        actual,
        ["@repo/lib#build", "js-pkg#build", "lib#build", "api#build"].into()
    );
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

    let api_hash = task_hash(tempdir.path(), "api", "build");
    let lib_hash = task_hash(tempdir.path(), "lib", "build");
    let independent_hash = task_hash(tempdir.path(), "independent", "build");
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
    assert!(
        tasks
            .iter()
            .any(|task| { task["name"] == "build" && task["package"]["name"] == "independent" })
    );
    assert_ne!(
        independent_hash,
        task_hash(tempdir.path(), "independent", "build"),
        "the changed module must be invalidated"
    );
    assert_eq!(
        api_hash,
        task_hash(tempdir.path(), "api", "build"),
        "an unrelated module must not invalidate the API"
    );
    assert_eq!(
        lib_hash,
        task_hash(tempdir.path(), "lib", "build"),
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
    let api_binary = tempdir.path().join("apps/api/api");
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

    let worker_binary = worker.join("worker");
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
        &["run", "lint", "--filter=lib", "--dry-run=json"],
    );
    let lint = dry_run_task(&output, "lib#lint");
    assert_eq!(lint["command"], "go version");

    let output = run_turbo(
        tempdir.path(),
        &["run", "dev", "--filter=api", "--dry-run=json"],
    );
    let dev = dry_run_task(&output, "api#dev");
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
    let tasks = package_task_names(tempdir.path(), "api");
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
            .join("apps/api")
            .join(if cfg!(windows) { "api.exe" } else { "api" })
            .exists(),
        "unfiltered native build must produce the runnable binary"
    );

    let filtered = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(filtered.path());
    let build_args = ["run", "build", "--filter=api", "--log-order=grouped"];
    let binary =
        filtered
            .path()
            .join("apps/api")
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
        stdout.contains("api:build: cache hit"),
        "second build must hit cache: {stdout}"
    );

    fs::remove_file(&binary).unwrap();
    let output = run_turbo(filtered.path(), &build_args);
    assert_command_success(&output, "restored filtered Go build");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("api:build: cache hit"),
        "restoration must come from cache: {stdout}"
    );
    assert!(binary.exists(), "cache hit must restore the binary");

    let output = run_turbo(
        filtered.path(),
        &["run", "dev", "--filter=api", "--", "passed-to-go"],
    );
    assert_command_success(&output, "native Go dev with pass-through argument");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("passed-to-go"),
        "go run must receive pass-through arguments: {output:?}"
    );
}

#[test]
fn test_go_versioned_default_binaries_match_go_and_do_not_hash_into_dependents() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    let root = tempdir.path();
    setup_go_pure_workspace(root);
    // Neither the module directory nor the final v2 import-path component is
    // the executable name. The library also has a sole nested main package.
    fs::write(
        root.join("apps/api/go.mod"),
        "module example.com/service/v2\n\ngo 1.22\n\nrequire example.com/lib/v2 v2.0.0\n\nreplace \
         example.com/lib/v2 => ../../packages/lib\n",
    )
    .unwrap();
    let main = root.join("apps/api/main.go");
    let source = fs::read_to_string(&main).unwrap();
    fs::write(
        main,
        source.replace("example.com/lib", "example.com/lib/v2"),
    )
    .unwrap();
    fs::write(
        root.join("packages/lib/go.mod"),
        "module example.com/lib/v2\n\ngo 1.22\n",
    )
    .unwrap();
    let nested = root.join("packages/lib/cmd/worker/v2");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("main.go"), "package main\n\nfunc main() {}\n").unwrap();

    let cases = [
        ("lib/v2#build", "packages/lib", "./cmd/worker/v2", "worker"),
        ("service/v2#build", "apps/api", ".", "service"),
    ];
    let binaries = cases.map(|(_, directory, _, name)| {
        root.join(directory)
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
    });
    let dry_run = || {
        let output = run_turbo(root, &["run", "build", "--dry-run=json"]);
        cases.map(|(task, _, _, _)| dry_run_task(&output, task))
    };
    let before = dry_run();
    assert!(
        before[1]["dependencies"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!(cases[0].0))
    );
    for (index, (_, directory, target, name)) in cases.iter().enumerate() {
        let task = &before[index];
        assert_eq!(task["command"], format!("go build {target}"));
        assert_eq!(task["resolvedTaskDefinition"]["cache"], true);
        assert_eq!(
            task["resolvedTaskDefinition"]["outputs"],
            serde_json::json!([format!("{name}{}", std::env::consts::EXE_SUFFIX)])
        );
        // Use Go itself as the oracle, without -o or changing into the target.
        let output = run_go(&root.join(directory), &["build", target]);
        assert_command_success(&output, "direct Go build with its default output name");
        assert!(
            binaries[index].is_file(),
            "Go must create {:?}",
            binaries[index]
        );
    }
    let assert_hashes_unchanged = || {
        let after = dry_run();
        for (index, (task, _, _, _)) in cases.iter().enumerate() {
            assert_eq!(
                before[index]["hash"], after[index]["hash"],
                "{task} must not hash generated binaries"
            );
        }
    };
    assert_hashes_unchanged();
    for binary in &binaries {
        fs::remove_file(binary).unwrap();
    }

    let mut cached_bytes = Vec::new();
    for (stage, status) in [
        ("cold", "cache miss"),
        ("warm", "cache hit"),
        ("restored", "cache hit"),
    ] {
        if stage == "restored" {
            // Changing either output must not invalidate its own task or the
            // downstream executable, including the dependency source closure.
            for binary in &binaries {
                fs::write(binary, "changed generated binary").unwrap();
            }
            assert_hashes_unchanged();
            for binary in &binaries {
                fs::remove_file(binary).unwrap();
            }
            assert_hashes_unchanged();
        }
        let output = run_turbo(root, &["run", "build", "--log-order=grouped"]);
        assert_command_success(&output, &format!("{stage} versioned Go build"));
        let combined = common::combined_output(&output);
        for (task, _, _, _) in cases {
            let expected = format!("{}:build: {status}", task.strip_suffix("#build").unwrap());
            assert!(
                combined.contains(&expected),
                "expected {expected}: {combined}"
            );
        }
        for (index, binary) in binaries.iter().enumerate() {
            let contents =
                fs::read(binary).expect("native binary must be present after every build");
            if stage == "cold" {
                cached_bytes.push(contents);
            } else {
                assert_eq!(
                    contents, cached_bytes[index],
                    "cached binary must be restored exactly"
                );
            }
            let output = std::process::Command::new(binary).output().unwrap();
            assert_command_success(&output, "execute native or restored Go binary");
        }
        assert_hashes_unchanged();
    }

    // Prove the dependency relationship is still hashed, rather than masking
    // all changes in the module containing the generated worker binary.
    fs::write(
        root.join("packages/lib/lib.go"),
        "package lib\n\nfunc Greet() { println(\"changed\") }\n",
    )
    .unwrap();
    let changed = dry_run();
    for (index, (task, _, _, _)) in cases.iter().enumerate() {
        assert_ne!(
            before[index]["hash"], changed[index]["hash"],
            "library source changes must invalidate {task}"
        );
    }
}

#[test]
fn test_go_explicit_build_command_preserves_authored_output_and_cache_restore() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    let root = tempdir.path();
    setup_go_pure_workspace(root);
    // Custom output names are not native binary exclusions. Declare the input
    // exclusion as well so the authored binary cannot invalidate its own cache.
    fs::write(
        root.join("apps/api/turbo.json"),
        r#"{
  "extends": ["//"],
  "tasks": {
    "build": {
      "command": ["go", "build", "-o", "bin/pigo-api", "."],
      "inputs": ["$TURBO_DEFAULT$", "!bin/pigo-api"],
      "outputs": ["bin/pigo-api"]
    }
  }
}"#,
    )
    .unwrap();
    let output = run_turbo(root, &["run", "build", "--filter=api", "--dry-run=json"]);
    let task = dry_run_task(&output, "api#build");
    assert_eq!(task["command"], "go build -o bin/pigo-api .");
    // Authored outputs are merged with inferred metadata, but the explicit
    // command must still write and restore the authored path, not the default.
    assert!(
        task["resolvedTaskDefinition"]["outputs"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("bin/pigo-api"))
    );
    assert_eq!(task["resolvedTaskDefinition"]["cache"], true);

    let binary = root.join("apps/api/bin/pigo-api");
    let native_binary = root
        .join("apps/api")
        .join(format!("api{}", std::env::consts::EXE_SUFFIX));
    assert_go_build_cache_result(root, &[], "cache miss", "cold authored Go build");
    let contents = fs::read(&binary).expect("explicit -o must preserve bin/pigo-api verbatim");
    assert_go_build_cache_result(root, &[], "cache hit", "warm authored Go build");
    fs::remove_file(&binary).unwrap();
    assert_go_build_cache_result(root, &[], "cache hit", "restored authored Go build");
    assert_eq!(fs::read(&binary).unwrap(), contents);
    assert!(
        !native_binary.exists(),
        "the native output must not shadow explicit -o"
    );
}

#[test]
fn test_go_format_runs_per_module_without_formatting_non_packages() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    let root = tempdir.path();
    setup_go_pure_workspace(root);
    let sources = [
        (
            "apps/api/format.go",
            "package main\nfunc   formatMe( ){ }\n",
            "package main\n\nfunc formatMe() {}\n",
        ),
        (
            "packages/lib/format.go",
            "package lib\nfunc   formatMe( ){ }\n",
            "package lib\n\nfunc formatMe() {}\n",
        ),
    ];
    let ignored_sources = [
        "packages/lib/testdata/example/ignored.go",
        "packages/lib/nested/ignored.go",
    ];
    let ignored_content = "package ignored\nfunc   untouched( ){ }\n";
    for path in ignored_sources {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, ignored_content).unwrap();
    }
    fs::write(
        root.join("packages/lib/nested/go.mod"),
        "module example.com/nested\n\ngo 1.22\n",
    )
    .unwrap();
    for (path, unformatted, _) in sources {
        fs::write(root.join(path), unformatted).unwrap();
    }

    let output = run_turbo(root, &["run", "format", "--filter=lib"]);
    assert_command_success(&output, "filtered native Go format");
    assert_eq!(
        fs::read_to_string(root.join(sources[0].0)).unwrap(),
        sources[0].1
    );
    assert_eq!(
        fs::read_to_string(root.join(sources[1].0)).unwrap(),
        sources[1].2
    );

    // Restore identical inputs before each run: source-mutating tasks must not
    // replay a cached success instead of formatting the files again.
    for _ in 0..2 {
        for (path, unformatted, _) in sources {
            fs::write(root.join(path), unformatted).unwrap();
        }
        let output = run_turbo(root, &["run", "format"]);
        assert_command_success(&output, "workspace-wide native Go format");
        for (path, _, formatted) in sources {
            assert_eq!(
                fs::read_to_string(root.join(path)).unwrap(),
                formatted,
                "{path}"
            );
        }
        for path in ignored_sources {
            assert_eq!(
                fs::read_to_string(root.join(path)).unwrap(),
                ignored_content,
                "format must leave non-package source untouched: {path}"
            );
        }
    }
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
    let output = run_turbo(tempdir.path(), &["run", "format", "--filter=lib"]);
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
    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=api"]);
    assert_command_success(&output, "authored Go build override");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("go version go"),
        "the authored command must execute: {output:?}"
    );
    assert!(
        !tempdir
            .path()
            .join("apps/api")
            .join(if cfg!(windows) { "api.exe" } else { "api" })
            .exists(),
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
    let tasks = package_task_names(tempdir.path(), "api");
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
    let output = run_turbo(tempdir.path(), &["run", "test", "--filter=lib"]);
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
fn test_go_summarized_execution_writes_portable_task_facts() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    let executable = if cfg!(windows) { "api.exe" } else { "api" };
    let output_path = executable;
    let build_command = "go build .";
    let task_directory = Path::new("apps").join("api").to_string_lossy().into_owned();

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
        .and_then(|tasks| tasks.iter().find(|task| task["taskId"] == "api#build"))
        .expect("summarized Go build task");
    assert_eq!(summarized_build["package"], "api");
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
            .is_some_and(|outputs| outputs.iter().any(|output| output == output_path))
    );
    assert!(
        summarized_build["hashOfExternalDependencies"]
            .as_str()
            .is_some_and(|hash| !hash.is_empty())
    );
}

#[test]
fn test_mixed_resolution_domains_cli_wiring_smoke() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_monorepo(tempdir.path());
    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "{ externalDependencies { items { name internalDependents { items { name } } } } }",
        ],
    );
    assert_command_success(&output, "mixed external dependency query");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let externals = json["data"]["externalDependencies"]["items"]
        .as_array()
        .expect("mixed external dependency query shape");
    // Keep one real discovery/serialization seam; exact domain membership is
    // asserted over injected resolution facts in the query crate.
    for (external, dependent) in [("go", "api"), ("picocolors@1.1.1", "js-pkg")] {
        let package = externals
            .iter()
            .find(|item| item["name"] == external)
            .unwrap_or_else(|| panic!("missing external {external}"));
        assert!(
            package["internalDependents"]["items"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item["name"] == dependent))
        );
    }
}

#[test]
fn test_go_regression_profile_outputs_are_not_log_only_cache_hits() {
    if !go_available() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let outputs = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(root.path());
    let profile = outputs.path().join("coverage.out");
    let flag = format!("-coverprofile={}", profile.display());
    // Cover both executable and library modules. The report deliberately lives
    // outside default input globs.
    for filter in ["--filter=api", "--filter=lib"] {
        let args = ["run", "test", filter, "--", &flag];
        for _ in 0..2 {
            let output = run_turbo(root.path(), &args);
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
        &["run", "test", "--filter=api"],
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
    setup_go_pure_workspace(root.path());
    let binary = outputs.path().join(if cfg!(windows) {
        "custom.exe"
    } else {
        "custom"
    });
    let output = run_turbo_with_env(
        root.path(),
        &[
            "run",
            "build",
            "--filter=api",
            "--",
            "-o",
            binary.to_str().unwrap(),
        ],
        &[],
    );
    assert_command_success(&output, "Go build with custom output");
    assert!(
        binary.exists(),
        "the native build must honor the user's explicit -o"
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
    for filter in [Some("--filter=api"), None] {
        let mut args = vec!["run", "test"];
        args.extend(filter);
        args.extend(["--", "-args", "-custom=expected"]);
        let output = run_turbo(root.path(), &args);
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
        &["run", "build", "--filter=api", "--dry-run=json"],
        &[("GOENV", goenv.to_str().unwrap())],
    );
    let build = dry_run_task(&output, "api#build");
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
            "--filter=api",
            "--dry-run=json",
            "--",
            "-overlay=elsewhere.json",
        ],
    );
    let build = dry_run_task(&output, "api#build");
    assert_eq!(
        build["resolvedTaskDefinition"]["cache"], false,
        "explicit outputs do not describe an overlay's untracked source inputs"
    );
}
