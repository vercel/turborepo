#![cfg(unix)]

mod common;

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::json;

struct Fixture {
    directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let jest = Command::new("node")
            .args(["-p", "require.resolve('jest/package.json')"])
            .current_dir(repo.join("packages/create-turbo"))
            .output()
            .unwrap();
        assert!(
            jest.status.success(),
            "install workspace dependencies first"
        );
        let jest = PathBuf::from(String::from_utf8(jest.stdout).unwrap().trim());
        fs::create_dir_all(root.join("node_modules/.bin")).unwrap();
        std::os::unix::fs::symlink(jest.parent().unwrap(), root.join("node_modules/jest")).unwrap();
        fs::copy(
            repo.join("packages/create-turbo/node_modules/.bin/jest"),
            root.join("node_modules/.bin/jest"),
        )
        .unwrap();
        fs::write(root.join("package.json"), json!({
            "name": "test-cache-fixture", "private": true, "packageManager": "npm@10.5.0", "workspaces": ["packages/*"]
        }).to_string()).unwrap();
        fs::write(
            root.join("package-lock.json"),
            "{\"name\":\"test-cache-fixture\",\"lockfileVersion\":3,\"packages\":{}}",
        )
        .unwrap();
        fs::write(root.join(".gitignore"), "node_modules\n.turbo\n").unwrap();
        for name in ["a", "b"] {
            fs::create_dir_all(root.join(format!("packages/{name}"))).unwrap();
            fs::write(
                root.join(format!("packages/{name}/package.json")),
                json!({
                    "name": name, "scripts": {"test":"jest --runInBand"},
                    "jest": {"transform":{}, "testEnvironment":"node"}
                })
                .to_string(),
            )
            .unwrap();
        }
        common::git(root, &["init", "--quiet"]);
        let fixture = Self { directory };
        fixture.config(true, None);
        fixture.write("packages/shared.js", "module.exports = 1;");
        fixture.write(
            "packages/a/shared.test.js",
            "test('shared', () => { console.log('EXECUTED_SHARED'); \
             expect(require('../shared')).toBe(1); });",
        );
        fixture.write(
            "packages/a/local.test.js",
            "test('local', () => { console.log('EXECUTED_LOCAL'); expect(2).toBe(2); });",
        );
        fixture.write(
            "packages/b/other.test.js",
            "test('other', () => { console.log('EXECUTED_OTHER'); expect(3).toBe(3); });",
        );
        fixture
    }

    fn root(&self) -> &Path {
        self.directory.path()
    }

    fn write(&self, path: &str, source: &str) {
        fs::write(self.root().join(path), source).unwrap();
    }

    fn config(&self, enabled: bool, command: Option<serde_json::Value>) {
        let mut task = json!({"outputs": []});
        if let Some(command) = command {
            task["command"] = command;
        }
        self.write("turbo.json", &json!({
            "futureFlags": {"experimentalTestCaching": enabled, "experimentalTaskCommand": true},
            "tasks": {"test": task}
        }).to_string());
    }

    fn run(&self, success: bool) -> String {
        let output = common::run_turbo(
            self.root(),
            &["run", "test", "--cache=local:rw", "--output-logs=full"],
        );
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.status.success(), success, "{text}");
        text
    }
}

struct Remote {
    url: String,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Remote {
    fn new(supported: bool) -> Self {
        use std::{
            io::{Read, Write},
            sync::{
                Arc, Mutex,
                atomic::{AtomicBool, Ordering},
            },
        };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let worker_stop = stop.clone();
        let worker_requests = requests.clone();
        let worker = std::thread::spawn(move || {
            let mut records = serde_json::Map::new();
            'connections: while !worker_stop.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("{error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                    .unwrap();
                let mut request = Vec::new();
                let (header_end, length) = loop {
                    let mut bytes = [0; 4096];
                    let n = match stream.read(&mut bytes) {
                        Ok(0) => continue 'connections,
                        Ok(n) => n,
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            continue 'connections;
                        }
                        Err(error) => panic!("{error}"),
                    };
                    request.extend_from_slice(&bytes[..n]);
                    if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .map(|n| n.parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        break (end + 4, length);
                    }
                };
                while request.len() < header_end + length {
                    let mut bytes = [0; 4096];
                    let n = stream.read(&mut bytes).unwrap();
                    assert_ne!(n, 0);
                    request.extend_from_slice(&bytes[..n]);
                }
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let first = headers.lines().next().unwrap().to_string();
                worker_requests.lock().unwrap().push(first.clone());
                let bulk = first.contains("/v8/artifacts/experimental-test-results");
                let body = if supported && bulk {
                    let payload: serde_json::Value =
                        serde_json::from_slice(&request[header_end..header_end + length]).unwrap();
                    if first.starts_with("PUT") {
                        records.extend(payload["results"].as_object().unwrap().clone());
                        json!({})
                    } else {
                        let hits: serde_json::Map<String, serde_json::Value> = payload["keys"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .filter_map(|key| {
                                records.get(key.as_str().unwrap()).map(|result| {
                                    (key.as_str().unwrap().to_string(), result.clone())
                                })
                            })
                            .collect();
                        serde_json::Value::Object(hits)
                    }
                } else {
                    json!({})
                }
                .to_string();
                let status = if supported && bulk {
                    "200 OK"
                } else {
                    "404 Not Found"
                };
                let capability = if supported && bulk {
                    "x-turbo-test-cache: 1\r\n"
                } else {
                    ""
                };
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\n{capability}Content-Type: \
                     application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        Self {
            url,
            stop,
            requests,
            worker: Some(worker),
        }
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let result = self.worker.take().unwrap().join();
        if !std::thread::panicking() {
            result.unwrap();
        }
    }
}

#[test]
fn remote_bulk_and_unsupported_service_run_real_tests() {
    for supported in [false, true] {
        let fixture = Fixture::new();
        let remote = Remote::new(supported);
        let run = |root: &Path| {
            let output = common::run_turbo(
                root,
                &[
                    "run",
                    "test",
                    "--cache=remote:rw",
                    &format!("--api={}", remote.url),
                    "--token=test-token",
                    "--team=team_test",
                ],
            );
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.status.success(), "{text}");
            text
        };
        let cold = run(fixture.root());
        assert!(
            cold.contains("EXECUTED_SHARED") && cold.contains("EXECUTED_OTHER"),
            "{cold}"
        );
        let warm = if supported {
            let checkout = Fixture::new();
            run(checkout.root())
        } else {
            run(fixture.root())
        };
        if supported {
            assert!(warm.contains("test cache: 2 hit, 0 miss"), "{warm}");
            assert!(!warm.contains("EXECUTED_"), "{warm}");
        } else {
            assert!(!warm.contains("test cache:"), "{warm}");
            assert!(
                warm.contains("EXECUTED_SHARED") && warm.contains("EXECUTED_OTHER"),
                "{warm}"
            );
        }
        let requests = remote.requests.lock().unwrap();
        let bulk: Vec<_> = requests
            .iter()
            .filter(|request| request.contains("experimental-test-results"))
            .collect();
        assert_eq!(
            bulk.iter().filter(|r| r.starts_with("POST")).count(),
            4,
            "{requests:?}"
        );
        assert_eq!(
            bulk.iter().filter(|r| r.starts_with("PUT")).count(),
            if supported { 2 } else { 0 },
            "{requests:?}"
        );
        if supported {
            assert_eq!(
                requests
                    .iter()
                    .filter(|request| !request.contains("/v8/artifacts/status"))
                    .count(),
                6,
                "no parent or per-file requests: {requests:?}"
            );
        }
        assert!(
            !requests.iter().any(
                |request| request.split('/').next_back().is_some_and(|part| part
                    .split(['?', ' '])
                    .next()
                    .unwrap_or_default()
                    .len()
                    == 64)
            ),
            "no per-unit fallback: {requests:?}"
        );
    }
}

#[test]
fn static_esm_imports_are_cached_and_invalidated() {
    let fixture = Fixture::new();
    fixture.write(
        "packages/a/package.json",
        &json!({
            "name": "a", "type": "module", "scripts": {"test": "jest --runInBand"},
            "jest": {"transform": {}, "testMatch": ["**/*.test.mjs"]}
        })
        .to_string(),
    );
    fixture.write("packages/a/value.mjs", "export const value = 2;");
    fixture.write(
        "packages/a/esm.test.mjs",
        "import { value } from './value.mjs'; test('esm', () => { console.log('EXECUTED_ESM'); \
         expect(value).toBe(2); });",
    );
    let run = || {
        let output = common::run_turbo_with_env(
            fixture.root(),
            &["run", "test", "--cache=local:rw"],
            &[("NODE_OPTIONS", "--experimental-vm-modules")],
        );
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success(), "{text}");
        text
    };
    assert!(run().contains("EXECUTED_ESM"));
    let warm = run();
    assert!(!warm.contains("EXECUTED_"), "{warm}");
    fixture.write("packages/a/value.mjs", "export const value = 2; // edit");
    let changed = run();
    assert!(
        changed.contains("a:test: test cache: 0 hit, 1 miss"),
        "{changed}"
    );
    assert!(
        changed.contains("EXECUTED_ESM") && !changed.contains("EXECUTED_OTHER"),
        "{changed}"
    );
}

#[test]
fn prerequisite_hashes_deliberately_invalidate_all_consumers() {
    let fixture = Fixture::new();
    fixture.write(
        "turbo.json",
        &json!({
            "futureFlags": {"experimentalTestCaching": true, "experimentalTaskCommand": true},
            "tasks": {
                "test": {"outputs": [], "dependsOn": ["prepare"]},
                "prepare": {"outputs": [], "command": ["node", "-e", "console.log('prepared')"]}
            }
        })
        .to_string(),
    );
    fixture.run(true);
    let warm = fixture.run(true);
    assert!(!warm.contains("EXECUTED_"), "{warm}");
    fixture.write("packages/a/unrelated.js", "module.exports = 1;");
    let changed = fixture.run(true);
    assert!(
        changed.contains("a:test: test cache: 0 hit, 2 miss"),
        "{changed}"
    );
    assert!(
        changed.contains("b:test: test cache: 1 hit, 0 miss"),
        "{changed}"
    );
    assert!(
        changed.contains("EXECUTED_SHARED") && changed.contains("EXECUTED_LOCAL"),
        "{changed}"
    );
}

#[test]
fn all_hit_summary_reports_local_cache_hits() {
    let fixture = Fixture::new();
    fixture.run(true);
    let output = common::run_turbo(
        fixture.root(),
        &["run", "test", "--cache=local:rw", "--summarize"],
    );
    assert!(output.status.success());
    let summaries: Vec<_> = fs::read_dir(fixture.root().join(".turbo/runs"))
        .unwrap()
        .collect();
    assert_eq!(summaries.len(), 1);
    let summary: serde_json::Value =
        serde_json::from_slice(&fs::read(summaries[0].as_ref().unwrap().path()).unwrap()).unwrap();
    for task in summary["tasks"].as_array().unwrap() {
        assert_eq!(task["cache"]["status"], "HIT", "{summary}");
        assert_eq!(task["cache"]["local"], true, "{summary}");
    }
}

#[test]
fn direct_launcher_override_preserves_identity() {
    let fixture = Fixture::new();
    let launcher = fixture.root().join("node_modules/.bin/jest");
    fixture.write(
        "turbo.json",
        &json!({
            "futureFlags": {"experimentalTestCaching": true, "experimentalTaskCommand": true},
            "tasks": {"test": {"outputs": []}, "a#test": {"command": [launcher, "--runInBand"]}}
        })
        .to_string(),
    );
    let cold = fixture.run(true);
    assert!(cold.contains("a:test: test cache: 0 hit, 2 miss"), "{cold}");
    assert!(
        fixture
            .run(true)
            .contains("a:test: test cache: 2 hit, 0 miss")
    );
    let mut shim = fs::read_to_string(&launcher).unwrap();
    shim = shim.replace("#!/bin/sh", "#!/bin/sh\necho ACTUAL_OVERRIDE\nexit 7");
    fs::write(&launcher, shim).unwrap();
    fixture.write(
        "packages/a/local.test.js",
        "test('changed', () => expect(1).toBe(1));",
    );
    let rejected = fixture.run(false);
    assert!(rejected.contains("ACTUAL_OVERRIDE"), "{rejected}");
    assert!(!rejected.contains("a:test: test cache:"), "{rejected}");
}

#[test]
fn direct_jest_target_does_not_require_a_package_manager_shim() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.root().join("node_modules/.bin/jest")).unwrap();
    fixture.write("turbo.json", &json!({
        "futureFlags": {"experimentalTestCaching": true, "experimentalTaskCommand": true},
        "tasks": {"test": {"outputs": []}, "a#test": {"command": [fixture.root().join("node_modules/jest/bin/jest.js"), "--runInBand"]}}
    }).to_string());
    for expected in ["0 hit, 2 miss", "2 hit, 0 miss"] {
        let output = common::run_turbo(
            fixture.root(),
            &["run", "test", "--filter=a", "--cache=local:rw"],
        );
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success(), "{text}");
        assert!(text.contains(expected), "{text}");
    }
}

#[test]
fn docblock_environment_changes_never_reuse_success() {
    let fixture = Fixture::new();
    fixture.write(
        "packages/a/environment.cjs",
        "const {createRequire} = require('node:module'); const {TestEnvironment} = \
         createRequire(require.resolve('jest/package.json'))('jest-environment-node'); \
         module.exports = class extends TestEnvironment {}; ",
    );
    fixture.write(
        "packages/a/local.test.js",
        "/** @jest-environment ./environment.cjs */\ntest('local', () => expect(2).toBe(2));",
    );
    fixture.run(true);
    fixture.run(true);
    fixture.write(
        "packages/a/environment.cjs",
        "throw new Error('CHANGED_ENVIRONMENT');",
    );
    let changed = fixture.run(false);
    assert!(changed.contains("CHANGED_ENVIRONMENT"), "{changed}");
    assert!(!changed.contains("a:test: test cache:"), "{changed}");
}

#[test]
fn entry_package_boundary_invalidates_cached_success() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.root().join("packages/a/nested")).unwrap();
    fixture.write(
        "packages/a/nested/entry.test.js",
        "module.exports = {}; test('entry', () => { console.log('EXECUTED_ENTRY'); \
         expect(1).toBe(1); });",
    );
    let run = |success| {
        let output = common::run_turbo_with_env(
            fixture.root(),
            &["run", "test", "--cache=local:rw"],
            &[("NODE_OPTIONS", "--experimental-vm-modules")],
        );
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.status.success(), success, "{text}");
        text
    };
    run(true);
    assert!(!run(true).contains("EXECUTED_ENTRY"));
    fixture.write("packages/a/nested/package.json", "{\"type\":\"module\"}");
    let changed = run(false);
    assert!(changed.contains("module is not defined"), "{changed}");
}

#[test]
fn external_config_is_not_evaluated_by_discovery() {
    for enabled in [false, true] {
        let fixture = Fixture::new();
        fixture.config(enabled, None);
        fixture.write(
            "packages/a/package.json",
            "{\"name\":\"a\",\"scripts\":{\"test\":\"jest --runInBand\"}}",
        );
        fixture.write(
            "packages/a/jest.config.cjs",
            "require('node:fs').appendFileSync(require('node:path').join(__dirname, \
             '../evaluations'), 'x'); module.exports = {transform: {}};",
        );
        fixture.run(true);
        assert_eq!(
            fs::read_to_string(fixture.root().join("packages/evaluations")).unwrap(),
            "x"
        );
        fixture.run(true);
        assert_eq!(
            fs::read_to_string(fixture.root().join("packages/evaluations")).unwrap(),
            "x"
        );
    }
}

#[test]
fn ancestor_config_declines_before_jest_config_resolution() {
    let fixture = Fixture::new();
    fixture.write(
        "packages/jest.config.cjs",
        "throw new Error('UNSELECTED_ANCESTOR');",
    );
    let cold = fixture.run(true);
    assert!(!cold.contains("test cache:"), "{cold}");
    assert!(cold.contains("EXECUTED_LOCAL"), "{cold}");
    let warm = fixture.run(true);
    assert!(!warm.contains("test cache:"), "{warm}");
    assert!(warm.contains("cache hit"), "{warm}");
}

#[test]
fn structured_inputs_keep_ordinary_task_cache() {
    for inputs in [
        json!([{"mode": "jit", "withDefaults": true}]),
        json!([{"mode": "dependencyOutputs", "from": ["prepare"], "globs": ["generated.txt"]}]),
    ] {
        let fixture = Fixture::new();
        fixture.write("turbo.json", &json!({
            "futureFlags": {"experimentalTestCaching": true, "experimentalTaskCommand": true},
            "tasks": {
                "test": {"outputs": [], "inputs": inputs, "dependsOn": ["prepare"]},
                "prepare": {"outputs": ["generated.txt"], "command": ["node", "-e", "require('fs').writeFileSync('generated.txt', 'generated')"]}
            }
        }).to_string());
        let cold = fixture.run(true);
        assert!(!cold.contains("test cache:"), "{cold}");
        assert!(cold.contains("EXECUTED_LOCAL"), "{cold}");
        let warm = fixture.run(true);
        assert!(!warm.contains("test cache:"), "{warm}");
        assert!(warm.contains("a:test: cache "), "{warm}");
    }
}

#[test]
fn explicit_inputs_keep_ordinary_task_cache() {
    let fixture = Fixture::new();
    fixture.write("turbo.json", &json!({
        "futureFlags": {"experimentalTestCaching": true},
        "tasks": {"test": {"outputs": [], "inputs": ["$TURBO_DEFAULT$", "$TURBO_ROOT$/extra.txt"]}}
    }).to_string());
    fixture.write("extra.txt", "first");
    let cold = fixture.run(true);
    assert!(!cold.contains("test cache:"), "{cold}");
    fixture.run(true);
    fixture.write("extra.txt", "second");
    let changed = fixture.run(true);
    assert!(changed.contains("EXECUTED_LOCAL"), "{changed}");
    assert!(!changed.contains("test cache:"), "{changed}");
}

#[test]
fn independent_jest_results_and_fallbacks() {
    let fixture = Fixture::new();
    let cold = fixture.run(true);
    assert!(cold.contains("test cache: 0 hit, 2 miss"), "{cold}");
    assert!(
        cold.contains("EXECUTED_SHARED") && cold.contains("EXECUTED_OTHER"),
        "{cold}"
    );
    assert_eq!(
        cold.matches("Ran all test suites").count(),
        2,
        "one batch per package: {cold}"
    );
    assert!(
        !fs::read_dir(fixture.root().join(".turbo/cache"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .path()
                .extension()
                .is_some_and(|ext| ext == "zst")),
        "unit results must not fabricate parent artifacts"
    );
    let warm = fixture.run(true);
    assert!(warm.contains("test cache: 2 hit, 0 miss"), "{warm}");
    assert!(!warm.contains("EXECUTED_"), "{warm}");

    fixture.write("packages/a/unrelated.js", "module.exports = 999;");
    let unrelated = fixture.run(true);
    assert!(
        unrelated.contains("test cache: 2 hit, 0 miss"),
        "{unrelated}"
    );
    assert!(!unrelated.contains("EXECUTED_"), "{unrelated}");

    fixture.write("packages/shared.js", "module.exports = 1; // shared edit");
    let shared = fixture.run(true);
    assert!(shared.contains("test cache: 1 hit, 1 miss"), "{shared}");
    assert!(shared.contains("EXECUTED_SHARED"), "{shared}");
    assert!(
        !shared.contains("EXECUTED_LOCAL") && !shared.contains("EXECUTED_OTHER"),
        "{shared}"
    );

    fixture.write(
        "packages/a/new.test.js",
        "test('new', () => { console.log('EXECUTED_NEW'); expect(1).toBe(1); });",
    );
    let new_test = fixture.run(true);
    assert!(new_test.contains("test cache: 2 hit, 1 miss"), "{new_test}");
    assert!(
        new_test.contains("EXECUTED_NEW") && !new_test.contains("EXECUTED_SHARED"),
        "{new_test}"
    );

    fixture.write(
        "packages/a/local.test.js",
        "test('local', () => { console.log('EXECUTED_FAILURE'); expect(1).toBe(2); });",
    );
    fixture.write(
        "packages/shared.js",
        "module.exports = 1; // another shared edit",
    );
    for _ in 0..2 {
        let failed = fixture.run(false);
        assert!(failed.contains("test cache: 1 hit, 2 miss"), "{failed}");
        assert!(
            failed.contains("EXECUTED_FAILURE") && failed.contains("EXECUTED_SHARED"),
            "{failed}"
        );
    }
    fixture.write(
        "packages/a/local.test.js",
        "test('local', () => expect(2).toBe(2));",
    );
    fixture.run(true);

    fixture.write(
        "packages/b/package.json",
        &json!({
            "name":"b", "scripts":{"test":"jest --runInBand"},
            "jest":{"transform":{}, "collectCoverage":true}
        })
        .to_string(),
    );
    let coverage = fixture.run(true);
    assert!(coverage.contains("EXECUTED_OTHER"), "{coverage}");
    assert_eq!(coverage.matches("test cache:").count(), 1, "{coverage}");

    fixture.config(false, None);
    let ordinary = fixture.run(true);
    assert!(!ordinary.contains("test cache:"), "{ordinary}");
    assert!(ordinary.contains("EXECUTED_SHARED"), "{ordinary}");

    fixture.config(true, Some(json!(["node", "-e", "process.exit(7)"])));
    let authored_script = fixture.run(true);
    assert!(authored_script.contains("test cache:"), "{authored_script}");

    fixture.write("turbo.json", &json!({
        "futureFlags": {"experimentalTestCaching": true, "experimentalTaskCommand": true},
        "tasks": {"test": {"outputs": []}, "a#test": {"command": ["node", "-e", "process.exit(7)"]}}
    }).to_string());
    let overridden = fixture.run(false);
    assert!(!overridden.contains("a:test: test cache:"), "{overridden}");
}
