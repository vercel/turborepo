#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path, process::Output};

use common::{combined_output, git, run_turbo};
use serde_json::{Value, json};

const A: &str = "check@packages%2Fapp-a%2Fa.mjs";
const B: &str = "check@packages%2Fapp-b%2Fb.mjs";

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "package.json",
        &json!({
            "name": "file-task-fixture", "private": true,
            "packageManager": "npm@10.5.0", "workspaces": ["packages/*"]
        })
        .to_string(),
    );
    write(
        dir.path(),
        ".gitignore",
        ".turbo\nnode_modules\n.executions\n",
    );
    write(
        dir.path(),
        "turbo.json",
        &json!({
            "agentGuidance": false,
            "futureFlags": { "experimentalTaskCommand": true },
            "tasks": { "//#check": {
                "experimentalFiles": { "include": ["packages/app-*/*.mjs"], "trace": true },
                "command": ["node", "$TURBO_FILE$"],
                "inputs": ["shared-setup.json"], "outputs": []
            }}
        })
        .to_string(),
    );
    write(dir.path(), "shared-setup.json", "{\"ready\":true}");
    for package in ["app-a", "app-b", "shared"] {
        write(
            dir.path(),
            &format!("packages/{package}/package.json"),
            &json!({"name":package,"version":"1.0.0"}).to_string(),
        );
    }
    write(
        dir.path(),
        "packages/shared/alpha.mjs",
        "export const value = 1;\n",
    );
    write(
        dir.path(),
        "packages/shared/beta.mjs",
        "export const value = 2;\n",
    );
    write(dir.path(), "packages/app-a/a.mjs", &unit("a", "alpha", 1));
    write(dir.path(), "packages/app-b/b.mjs", &unit("b", "beta", 2));
    git(dir.path(), &["init", "--quiet"]);
    git(dir.path(), &["config", "user.email", "test@example.com"]);
    git(dir.path(), &["config", "user.name", "Test"]);
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-qm", "fixture"]);
    dir
}

fn write(root: &Path, path: &str, text: &str) {
    let target = root.join(path);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(target, text).unwrap();
}

fn unit(name: &str, module: &str, expected: i32) -> String {
    format!(
        r#"import {{ appendFileSync, readFileSync }} from 'node:fs';
import assert from 'node:assert/strict';
import {{ value }} from '../shared/{module}.mjs';
appendFileSync('.executions', '{name}\n');
assert.equal(value, {expected});
assert.equal(JSON.parse(readFileSync('shared-setup.json', 'utf8')).ready, true);
"#
    )
}

fn execute(root: &Path, task: &str) -> Output {
    run_turbo(
        root,
        &[
            "run",
            task,
            "--cache=local:rw",
            "--output-logs=errors-only",
            "--concurrency=1",
        ],
    )
}

fn succeeds(output: &Output) {
    assert!(output.status.success(), "{}", combined_output(output));
}

fn executions(root: &Path) -> Vec<String> {
    fs::read_to_string(root.join(".executions"))
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

fn clear_executions(root: &Path) {
    fs::write(root.join(".executions"), "").unwrap();
}

fn configure(root: &Path, update: impl FnOnce(&mut Value)) {
    let path = root.join("turbo.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    update(&mut config["tasks"]["//#check"]);
    fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
}

fn bun_fixture() -> tempfile::TempDir {
    let dir = fixture();
    configure(dir.path(), |task| {
        task["experimentalFiles"]["resolution"] = json!("bun");
        task["command"] = json!(["bun", "$TURBO_FILE$"]);
    });
    dir
}

#[test]
fn aggregate_arguments_fail_instead_of_silently_running_unfiltered_units() {
    let dir = fixture();
    let output = run_turbo(dir.path(), &["run", "check", "--", "--filtered"]);
    assert!(!output.status.success());
    assert!(
        combined_output(&output).contains("individual"),
        "{}",
        combined_output(&output)
    );
    assert!(executions(dir.path()).is_empty());
}

#[test]
fn only_cannot_bypass_file_execution_or_traced_inputs() {
    for task in ["check", A] {
        let dir = bun_fixture();
        let output = run_turbo(dir.path(), &["run", task, "--only"]);
        assert!(!output.status.success());
        assert!(
            combined_output(&output).contains("--only cannot skip"),
            "{}",
            combined_output(&output)
        );
        assert!(executions(dir.path()).is_empty());
    }
}

#[test]
fn inherited_templates_cannot_execute_paths_from_another_package() {
    let dir = fixture();
    write(dir.path(), "turbo.json", &json!({"agentGuidance":false,"futureFlags":{"experimentalTaskCommand":true},"tasks":{"local-check":{}}}).to_string());
    write(
        dir.path(),
        "packages/app-a/turbo.json",
        &json!({
            "extends":["//"], "tasks":{"local-check":{
                "experimentalFiles":{"include":["a.mjs"]},
                "command":["node","$TURBO_FILE$"], "inputs":[]
            }}
        })
        .to_string(),
    );
    write(
        dir.path(),
        "packages/app-b/turbo.json",
        &json!({"extends":["//","app-a"]}).to_string(),
    );
    let output = run_turbo(dir.path(), &["run", "app-b#local-check"]);
    assert!(!output.status.success());
    assert!(
        combined_output(&output).contains("not inherited from another package"),
        "{}",
        combined_output(&output)
    );
    assert!(executions(dir.path()).is_empty());
}

#[test]
fn inherited_defaults_cannot_disappear_from_generated_children() {
    for name in ["local-check", "local-check@nested"] {
        let dir = fixture();
        let mut root = json!({"agentGuidance":false,"futureFlags":{"experimentalTaskCommand":true},"tasks":{"local-check":{}}});
        root["tasks"][name] = json!({"env":["EXPECTED"]});
        write(dir.path(), "turbo.json", &root.to_string());
        let template = json!({
            "experimentalFiles":{"include":["a.mjs"]},
            "command":["node","$TURBO_FILE$"], "inputs":[]
        });
        let mut package = json!({"extends":["//"],"tasks":{"local-check":template.clone()}});
        package["tasks"][name] = template;
        write(
            dir.path(),
            "packages/app-a/turbo.json",
            &package.to_string(),
        );
        let output = run_turbo(dir.path(), &["run", &format!("app-a#{name}")]);
        assert!(!output.status.success());
        assert!(
            combined_output(&output).contains("without inherited task defaults"),
            "{}",
            combined_output(&output)
        );
        assert!(executions(dir.path()).is_empty());
    }
}

#[test]
fn global_exclusions_cannot_remove_mandatory_entry_inputs() {
    let dir = fixture();
    let path = dir.path().join("turbo.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    config["futureFlags"]["globalConfiguration"] = json!(true);
    config["global"] = json!({"inputs":["!**/*.mjs"]});
    fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
    let output = execute(dir.path(), "check");
    assert!(!output.status.success());
    assert!(
        combined_output(&output).contains("global input exclusions"),
        "{}",
        combined_output(&output)
    );
    assert!(executions(dir.path()).is_empty());
}

#[test]
fn relocated_root_configs_cannot_hash_and_execute_different_files() {
    let dir = fixture();
    write(
        dir.path(),
        "configuration/turbo.json",
        &json!({
            "agentGuidance":false, "futureFlags":{"experimentalTaskCommand":true},
            "tasks":{"//#check":{
                "experimentalFiles":{"include":["entry.mjs"]},
                "command":["node","$TURBO_FILE$"], "inputs":[]
            }}
        })
        .to_string(),
    );
    write(
        dir.path(),
        "configuration/entry.mjs",
        "throw new Error('configured entry');\n",
    );
    write(
        dir.path(),
        "entry.mjs",
        "import { appendFileSync } from 'node:fs'; appendFileSync('.executions', 'wrong \
         entry\\n');\n",
    );
    let output = run_turbo(
        dir.path(),
        &["run", "check", "--root-turbo-json=configuration/turbo.json"],
    );
    assert!(!output.status.success());
    assert!(
        combined_output(&output).contains("root templates must be in the repository root"),
        "{}",
        combined_output(&output)
    );
    assert!(executions(dir.path()).is_empty());
}

#[test]
fn bun_aliases_trace_transitive_sources_and_extended_configurations() {
    let dir = bun_fixture();
    write(
        dir.path(),
        "packages/app-a/tsconfig.json",
        r#"{"extends":"./compiler.json"}"#,
    );
    write(
        dir.path(),
        "packages/app-a/compiler.json",
        r#"{"compilerOptions":{"paths":{"~/value":["../shared/alpha.mjs"]}}}"#,
    );
    write(
        dir.path(),
        "packages/app-a/a.mjs",
        &unit("a", "alpha", 1).replace("../shared/alpha.mjs", "~/value"),
    );
    succeeds(&execute(dir.path(), "check"));
    clear_executions(dir.path());
    write(
        dir.path(),
        "packages/shared/alpha.mjs",
        "export const value = 1; // traced edit\n",
    );
    succeeds(&execute(dir.path(), "check"));
    assert_eq!(executions(dir.path()), ["a"]);
    clear_executions(dir.path());
    write(
        dir.path(),
        "packages/app-a/compiler.json",
        r#"{"compilerOptions":{"paths":{"~/value":["../shared/alpha.mjs"]},"strict":true}}"#,
    );
    succeeds(&execute(dir.path(), "check"));
    assert_eq!(executions(dir.path()), ["a"]);
}

#[test]
fn function_type_checks_do_not_count_as_dynamic_code_loading() {
    let dir = bun_fixture();
    write(
        dir.path(),
        "packages/app-a/a.mjs",
        &(unit("a", "alpha", 1) + "assert.equal(value.constructor instanceof Function, true);\n"),
    );
    succeeds(&execute(dir.path(), A));
    assert_eq!(executions(dir.path()), ["a"]);
}

#[test]
fn runtime_resolution_overrides_require_explicit_inputs() {
    for flag in [
        "--main-fields=module,main",
        "--preserve-symlinks",
        "--preserve-symlinks-main",
        "--jsx-import-source=./custom-runtime",
        "-l.mjs:js",
        "-dDEBUG:true",
    ] {
        let dir = bun_fixture();
        configure(dir.path(), |task| {
            task["command"] = json!(["bun", flag, "$TURBO_FILE$"]);
        });
        let output = execute(dir.path(), A);
        assert!(!output.status.success(), "accepted {flag}");
        assert!(
            combined_output(&output).contains("custom module resolution flags"),
            "{flag}: {}",
            combined_output(&output)
        );
        assert!(executions(dir.path()).is_empty());
    }
}

#[cfg(unix)]
#[test]
fn implicit_jsx_runtime_edits_invalidate_cached_passes() {
    for expression in ["<item />", "<></>"] {
        let dir = bun_fixture();
        configure(dir.path(), |task| {
            task["experimentalFiles"]["include"] = json!(["packages/app-a/*.tsx"]);
        });
        write(
            dir.path(),
            "packages/react/package.json",
            r#"{"name":"react","exports":{"./jsx-runtime":"./runtime.ts","./jsx-dev-runtime":"./runtime.ts"}}"#,
        );
        fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        std::os::unix::fs::symlink("../packages/react", dir.path().join("node_modules/react"))
            .unwrap();
        let runtime = "export const Fragment = Symbol.for('fragment'); export const jsx = () => \
                       ({runtime: 1}); export const jsxs = jsx; export const jsxDEV = jsx;";
        write(dir.path(), "packages/react/runtime.ts", runtime);
        write(
            dir.path(),
            "packages/app-a/a.tsx",
            &"import {appendFileSync} from 'node:fs'; import assert from 'node:assert/strict'; \
              appendFileSync('.executions', 'a\\n'); const node = <><item /></>; \
              assert.equal(node.runtime, 1);"
                .replace("<><item /></>", expression),
        );
        succeeds(&execute(dir.path(), "check"));
        assert_eq!(executions(dir.path()), ["a"]);
        clear_executions(dir.path());
        succeeds(&execute(dir.path(), "check"));
        assert!(executions(dir.path()).is_empty());
        write(
            dir.path(),
            "packages/react/runtime.ts",
            &runtime.replace("runtime: 1", "runtime: 99"),
        );
        let output = execute(dir.path(), "check");
        assert!(!output.status.success(), "reused stale JSX runtime result");
        assert!(combined_output(&output).contains("99"));
        assert_eq!(executions(dir.path()), ["a"]);
    }
}

#[cfg(unix)]
#[test]
fn workspace_exports_trace_only_the_imported_source() {
    let dir = bun_fixture();
    write(
        dir.path(),
        "packages/shared/package.json",
        r#"{"name":"shared","exports":{"./alpha":"./alpha.mjs","./beta":"./beta.mjs"}}"#,
    );
    fs::create_dir_all(dir.path().join("node_modules")).unwrap();
    std::os::unix::fs::symlink("../packages/shared", dir.path().join("node_modules/shared"))
        .unwrap();
    write(
        dir.path(),
        "packages/app-a/a.mjs",
        &unit("a", "alpha", 1).replace("../shared/alpha.mjs", "shared/alpha"),
    );
    succeeds(&execute(dir.path(), "check"));
    clear_executions(dir.path());
    write(
        dir.path(),
        "packages/shared/alpha.mjs",
        "export const value = 1; // export edit\n",
    );
    succeeds(&execute(dir.path(), "check"));
    assert_eq!(executions(dir.path()), ["a"]);
}

#[test]
fn adding_a_higher_priority_extension_invalidates_a_cached_pass() {
    let dir = bun_fixture();
    write(
        dir.path(),
        "packages/app-a/a.mjs",
        &unit("a", "alpha", 1).replace("../shared/alpha.mjs", "../shared/alpha"),
    );
    succeeds(&execute(dir.path(), "check"));
    clear_executions(dir.path());
    write(
        dir.path(),
        "packages/shared/alpha.ts",
        "export const value = 99;\n",
    );
    let output = execute(dir.path(), "check");
    assert!(!output.status.success());
    assert!(
        combined_output(&output).contains("99"),
        "{}",
        combined_output(&output)
    );
    assert_eq!(executions(dir.path()), ["a"]);
}

#[test]
fn literal_dynamic_imports_in_nested_functions_track_their_sources() {
    for dir in [fixture(), bun_fixture()] {
        write(
            dir.path(),
            "packages/app-a/a.mjs",
            &unit("a", "alpha", 1).replace(
                "import { value } from '../shared/alpha.mjs';",
                "const load = () => import('../shared/alpha.mjs'); const { value } = await load();",
            ),
        );
        succeeds(&execute(dir.path(), "check"));
        clear_executions(dir.path());
        write(
            dir.path(),
            "packages/shared/alpha.mjs",
            "export const value = 99;\n",
        );
        let output = execute(dir.path(), "check");
        assert!(!output.status.success());
        assert!(
            combined_output(&output).contains("99"),
            "{}",
            combined_output(&output)
        );
        assert_eq!(executions(dir.path()), ["a"]);
    }
}

#[test]
fn preload_roots_track_transitive_inputs_even_when_entries_do_not_import_them() {
    let dir = bun_fixture();
    write(
        dir.path(),
        "preload.mjs",
        "import { value } from './packages/shared/setup.mjs'; if (value !== 3) throw new \
         Error('preload changed');\n",
    );
    write(
        dir.path(),
        "packages/shared/setup.mjs",
        "export const value = 3;\n",
    );
    configure(dir.path(), |task| {
        task["experimentalFiles"]["traceRoots"] = json!(["preload.mjs"]);
        task["command"] = json!(["bun", "--preload", "./preload.mjs", "$TURBO_FILE$"]);
    });
    succeeds(&execute(dir.path(), A));
    clear_executions(dir.path());
    write(
        dir.path(),
        "packages/shared/setup.mjs",
        "export const value = 4;\n",
    );
    let output = execute(dir.path(), A);
    assert!(!output.status.success());
    assert!(
        combined_output(&output).contains("preload changed"),
        "{}",
        combined_output(&output)
    );
    assert!(executions(dir.path()).is_empty());
}

#[test]
fn computed_imports_use_declared_fallback_inputs_without_hiding_source_changes() {
    let dir = bun_fixture();
    configure(dir.path(), |task| {
        task["experimentalFiles"]["fallbackInputs"] = json!(["packages/shared/**"]);
    });
    write(
        dir.path(),
        "packages/app-a/a.mjs",
        &unit("a", "alpha", 1).replace(
            "import { value } from '../shared/alpha.mjs';",
            "const name = '../shared/alpha.mjs'; const { value } = await import(name);",
        ),
    );
    succeeds(&execute(dir.path(), "check"));
    clear_executions(dir.path());
    write(
        dir.path(),
        "packages/shared/alpha.mjs",
        "export const value = 99;\n",
    );
    let output = execute(dir.path(), "check");
    assert!(!output.status.success());
    assert!(
        combined_output(&output).contains("99"),
        "{}",
        combined_output(&output)
    );
    assert_eq!(executions(dir.path()), ["a"]);
}

#[test]
fn cyclic_module_imports_share_inputs_without_cycling_the_execution_graph() {
    let dir = bun_fixture();
    write(
        dir.path(),
        "packages/shared/alpha.mjs",
        "import { value as leaf } from './leaf.mjs'; export const value = leaf;\n",
    );
    write(
        dir.path(),
        "packages/shared/leaf.mjs",
        "import './alpha.mjs'; export const value = 1;\n",
    );
    succeeds(&execute(dir.path(), "check"));
    clear_executions(dir.path());
    write(
        dir.path(),
        "packages/shared/leaf.mjs",
        "import './alpha.mjs'; export const value = 99;\n",
    );
    let output = execute(dir.path(), "check");
    assert!(!output.status.success());
    assert!(
        combined_output(&output).contains("99"),
        "{}",
        combined_output(&output)
    );
    assert_eq!(executions(dir.path()), ["a"]);
}

#[test]
fn git_task_filters_select_consumers_of_changed_source_inputs() {
    let dir = bun_fixture();
    let path = dir.path().join("turbo.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    config["futureFlags"]["filterUsingTasks"] = json!(true);
    fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
    git(dir.path(), &["add", "."]);
    git(
        dir.path(),
        &["commit", "-qm", "configure input-aware filtering"],
    );
    succeeds(&execute(dir.path(), "check"));
    clear_executions(dir.path());
    write(
        dir.path(),
        "packages/shared/alpha.mjs",
        "export const value = 99;\n",
    );
    let output = run_turbo(
        dir.path(),
        &["run", "check", "--filter=[HEAD]", "--cache=local:rw"],
    );
    assert!(!output.status.success());
    assert!(
        combined_output(&output).contains("99"),
        "{}",
        combined_output(&output)
    );
    assert_eq!(executions(dir.path()), ["a"]);
}

#[test]
fn declared_environment_changes_invalidate_file_results() {
    let dir = fixture();
    configure(dir.path(), |task| {
        task["env"] = json!(["FILE_TASK_EXPECTED"]);
    });
    write(
        dir.path(),
        "packages/app-a/a.mjs",
        "import { appendFileSync } from 'node:fs'; appendFileSync('.executions', \
         process.env.FILE_TASK_EXPECTED + '\\n'); if (process.env.FILE_TASK_EXPECTED !== 'good') \
         throw new Error('environment changed');\n",
    );
    let run = |value: &str| {
        common::turbo_command(dir.path())
            .args(["run", A, "--cache=local:rw"])
            .env("FILE_TASK_EXPECTED", value)
            .output()
            .unwrap()
    };
    succeeds(&run("good"));
    clear_executions(dir.path());
    let output = run("bad");
    assert!(!output.status.success());
    assert!(combined_output(&output).contains("environment changed"));
    assert_eq!(executions(dir.path()), ["bad"]);
}

fn dry(root: &Path, task: &str) -> Value {
    let output = run_turbo(root, &["run", task, "--dry=json", "--cache=local:rw"]);
    succeeds(&output);
    serde_json::from_slice(&output.stdout).unwrap()
}

fn child<'a>(summary: &'a Value, name: &str) -> &'a Value {
    summary["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["taskId"] == format!("//#{name}"))
        .unwrap()
}

#[test]
fn focused_runs_seed_full_runs_and_full_runs_seed_focused_runs() {
    for dir in [fixture(), bun_fixture()] {
        succeeds(&execute(dir.path(), A));
        assert_eq!(executions(dir.path()), ["a"]);
        let focused = dry(dir.path(), A);
        let full = dry(dir.path(), "check");
        assert_eq!(child(&focused, A)["hash"], child(&full, A)["hash"]);
        assert_eq!(child(&full, A)["cache"]["status"], "HIT");
        assert_eq!(child(&full, B)["cache"]["status"], "MISS");
        clear_executions(dir.path());
        succeeds(&execute(dir.path(), "check"));
        assert_eq!(executions(dir.path()), ["b"]);
        clear_executions(dir.path());
        succeeds(&execute(dir.path(), B));
        assert!(executions(dir.path()).is_empty());
    }
}

#[test]
fn focused_tasks_trace_only_the_reachable_execution_graph() {
    for dir in [fixture(), bun_fixture()] {
        // This unrelated entry cannot be traced. A focused run must neither
        // inspect it nor silently accept it when the full family is selected.
        write(
            dir.path(),
            "packages/app-b/b.mjs",
            "const name = '../shared/beta.mjs'; await import(name);\n",
        );
        let path = dir.path().join("turbo.json");
        let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        config["tasks"]["//#focused"] = json!({"command": null, "dependsOn": [A]});
        fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
        succeeds(&execute(dir.path(), "focused"));
        assert_eq!(executions(dir.path()), ["a"]);
        clear_executions(dir.path());
        succeeds(&execute(dir.path(), A));
        assert!(executions(dir.path()).is_empty());
        let output = execute(dir.path(), "check");
        assert!(!output.status.success());
        let message = combined_output(&output);
        assert!(
            message.contains("dynamic") || message.contains("computed"),
            "{message}"
        );
        assert!(executions(dir.path()).is_empty());
    }
}

#[test]
fn lazy_source_nodes_cannot_override_authored_task_definitions() {
    let dir = bun_fixture();
    let path = dir.path().join("turbo.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    config["tasks"]["//#check@source@packages%2Fshared%2Falpha.mjs"] =
        json!({"command": null, "inputs": [], "outputs": []});
    fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
    let output = execute(dir.path(), A);
    assert!(!output.status.success());
    let message = combined_output(&output);
    assert!(
        message.contains("generated task collides with"),
        "{message}"
    );
    assert!(
        message.contains("check@source@packages%2Fshared%2Falpha.mjs"),
        "{message}"
    );
    assert!(executions(dir.path()).is_empty());
}

#[test]
fn test_only_edits_do_not_invalidate_unrelated_consumers() {
    let dir = fixture();
    succeeds(&execute(dir.path(), "check"));
    clear_executions(dir.path());
    write(
        dir.path(),
        "packages/shared/private.test.mjs",
        "throw new Error('private test');\n",
    );
    write(
        dir.path(),
        "packages/app-a/a.mjs",
        &(unit("a", "alpha", 1) + "// changed check\n"),
    );
    succeeds(&execute(dir.path(), "check"));
    assert_eq!(executions(dir.path()), ["a"]);
}

#[test]
fn source_edits_invalidate_only_transitive_consumers() {
    let dir = fixture();
    write(
        dir.path(),
        "packages/shared/alpha.mjs",
        "export { value } from './leaf.mjs';\n",
    );
    write(
        dir.path(),
        "packages/shared/leaf.mjs",
        "export const value = 1;\n",
    );
    succeeds(&execute(dir.path(), "check"));
    clear_executions(dir.path());
    write(
        dir.path(),
        "packages/shared/leaf.mjs",
        "export const value = 1; // changed dependency\n",
    );
    succeeds(&execute(dir.path(), "check"));
    assert_eq!(executions(dir.path()), ["a"]);
}

#[test]
fn declared_shared_input_changes_invalidate_every_consumer() {
    let dir = fixture();
    succeeds(&execute(dir.path(), "check"));
    clear_executions(dir.path());
    write(
        dir.path(),
        "shared-setup.json",
        "{\"ready\":true,\"revision\":2}",
    );
    succeeds(&execute(dir.path(), "check"));
    let mut observed = executions(dir.path());
    observed.sort();
    assert_eq!(observed, ["a", "b"]);
}

#[test]
fn failed_units_are_reexecuted_and_make_the_aggregate_fail() {
    let dir = fixture();
    succeeds(&execute(dir.path(), "check"));
    write(
        dir.path(),
        "packages/shared/alpha.mjs",
        "export const value = 99;\n",
    );
    for _ in 0..2 {
        clear_executions(dir.path());
        let output = execute(dir.path(), "check");
        assert!(!output.status.success());
        assert!(
            combined_output(&output).contains("99 !== 1"),
            "{}",
            combined_output(&output)
        );
        assert_eq!(executions(dir.path()), ["a"]);
    }
}

#[test]
fn discovery_adds_new_units_without_invalidating_existing_units() {
    let dir = fixture();
    succeeds(&execute(dir.path(), "check"));
    clear_executions(dir.path());
    write(
        dir.path(),
        "packages/app-a/new.mjs",
        &unit("new", "alpha", 1),
    );
    succeeds(&execute(dir.path(), "check"));
    assert_eq!(executions(dir.path()), ["new"]);
    fs::remove_file(dir.path().join("packages/app-a/new.mjs")).unwrap();
    clear_executions(dir.path());
    succeeds(&execute(dir.path(), "check"));
    assert!(executions(dir.path()).is_empty());
}

#[test]
fn discovery_excludes_files_that_need_a_different_runner() {
    let dir = fixture();
    let config_path = dir.path().join("turbo.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["tasks"]["//#check"]["experimentalFiles"]["exclude"] = json!(["**/*.dom.mjs"]);
    fs::write(config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    write(
        dir.path(),
        "packages/app-a/other.dom.mjs",
        "throw new Error('requires a DOM runner');\n",
    );
    succeeds(&execute(dir.path(), "check"));
    let mut observed = executions(dir.path());
    observed.sort();
    assert_eq!(observed, ["a", "b"]);
    let summary = dry(dir.path(), "check");
    assert!(
        summary["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|task| { !task["taskId"].as_str().unwrap().contains("other.dom.mjs") })
    );
}

#[test]
fn unsupported_dynamic_imports_fail_before_a_cached_result_can_be_reused() {
    let dir = fixture();
    succeeds(&execute(dir.path(), "check"));
    clear_executions(dir.path());
    write(
        dir.path(),
        "packages/app-a/a.mjs",
        "const name = '../shared/alpha.mjs'; await import(name);\n",
    );
    let output = execute(dir.path(), "check");
    assert!(!output.status.success());
    let message = combined_output(&output);
    assert!(
        message.contains("dynamic")
            && message.contains("import/require")
            && message.contains("trace:false"),
        "{message}"
    );
    assert!(executions(dir.path()).is_empty());
}

#[test]
fn recoverable_parse_errors_cannot_seed_incomplete_cache_entries() {
    let dir = fixture();
    write(
        dir.path(),
        "packages/shared/invalid.mts",
        "export const value = <number>1;\n",
    );
    write(
        dir.path(),
        "packages/app-a/a.mjs",
        "import { value } from '../shared/invalid.mts';\n",
    );
    let output = run_turbo(dir.path(), &["run", A, "--dry=json"]);
    assert!(!output.status.success());
    let message = combined_output(&output);
    assert!(
        message.contains("Invalid experimental file task"),
        "{message}"
    );
    assert!(message.contains("This syntax is reserved"), "{message}");
}

#[test]
fn explicit_resolution_rejects_implicit_and_opaque_imports() {
    for (extension, source, diagnostic) in [
        ("jsx", "export const value = <div />;", "JSX requires"),
        (
            "mjs",
            "export const value = eval('1');",
            "not statically traceable",
        ),
        (
            "mjs",
            "export const value = new Function('return 1')();",
            "not statically traceable",
        ),
    ] {
        let dir = fixture();
        write(
            dir.path(),
            &format!("packages/shared/hidden.{extension}"),
            source,
        );
        write(
            dir.path(),
            "packages/app-a/a.mjs",
            &format!("import {{ value }} from '../shared/hidden.{extension}';\n"),
        );
        let output = run_turbo(dir.path(), &["run", A, "--dry=json"]);
        assert!(!output.status.success());
        let message = combined_output(&output);
        assert!(
            message.contains("Invalid experimental file task"),
            "{message}"
        );
        assert!(message.contains(diagnostic), "{message}");
    }
}

#[test]
fn declared_file_units_work_without_a_language_specific_runner() {
    let dir = fixture();
    write(dir.path(), "data/first.txt", "first");
    write(dir.path(), "data/second.txt", "second");
    write(
        dir.path(),
        "process.mjs",
        "import { readFileSync, appendFileSync } from 'node:fs'; appendFileSync('.executions', \
         readFileSync(process.argv[2], 'utf8') + '\\n');\n",
    );
    write(
        dir.path(),
        "turbo.json",
        &json!({
            "agentGuidance": false,
            "futureFlags": { "experimentalTaskCommand": true },
            "tasks": { "//#process": {
                "experimentalFiles": { "include": ["data/*.txt"] },
                "command": ["node", "process.mjs", "$TURBO_FILE$"],
                "inputs": ["process.mjs"], "outputs": []
            }}
        })
        .to_string(),
    );
    succeeds(&execute(dir.path(), "process@data%2Ffirst.txt"));
    assert_eq!(executions(dir.path()), ["first"]);
    clear_executions(dir.path());
    succeeds(&execute(dir.path(), "process"));
    assert_eq!(executions(dir.path()), ["second"]);
    clear_executions(dir.path());
    write(dir.path(), "data/first.txt", "modified");
    succeeds(&execute(dir.path(), "process"));
    assert_eq!(executions(dir.path()), ["modified"]);
}

#[test]
fn filenames_are_literal_arguments_and_literal_hash_inputs() {
    let dir = fixture();
    let filename = "packages/app-a/a space [x].mjs";
    write(dir.path(), filename, &unit("special", "alpha", 1));
    succeeds(&execute(dir.path(), "check"));
    clear_executions(dir.path());
    succeeds(&execute(dir.path(), "check"));
    assert!(executions(dir.path()).is_empty());
    write(
        dir.path(),
        filename,
        &(unit("special", "alpha", 1) + "// edit\n"),
    );
    succeeds(&execute(dir.path(), "check"));
    assert_eq!(executions(dir.path()), ["special"]);
}

#[test]
fn escaped_discovery_patterns_select_literal_route_names() {
    let dir = fixture();
    let config_path = dir.path().join("turbo.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["tasks"]["//#check"]["experimentalFiles"]["include"] =
        json!([r"packages/app-a/\(route\)/a \[x\].mjs"]);
    fs::write(config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    write(
        dir.path(),
        "packages/app-a/(route)/a [x].mjs",
        "import { appendFileSync } from 'node:fs'; appendFileSync('.executions', 'literal \
         route\\n');\n",
    );
    succeeds(&execute(dir.path(), "check"));
    assert_eq!(executions(dir.path()), ["literal route"]);
}

#[test]
fn deeply_nested_files_execute_and_restore_distinct_cached_logs() {
    let dir = fixture();
    let config_path = dir.path().join("turbo.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["tasks"]["//#check"]["experimentalFiles"]["include"] =
        json!(["packages/app-*/**/*.mjs"]);
    fs::write(config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let nested = format!(
        "packages/app-a/{}/{}/{}",
        "a".repeat(80),
        "b".repeat(80),
        "c".repeat(80)
    );
    for label in ["first", "second"] {
        write(
            dir.path(),
            &format!("{nested}/{label}.mjs"),
            &format!(
                "import {{ appendFileSync }} from 'node:fs';\nappendFileSync('.executions', \
                 '{label}\\n');\nconsole.log('{label} output');\n"
            ),
        );
    }
    succeeds(&execute(dir.path(), "check"));
    let mut observed = executions(dir.path());
    observed.sort();
    assert_eq!(observed, ["a", "b", "first", "second"]);
    let summary = dry(dir.path(), "check");
    let logs: Vec<_> = summary["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|task| task["taskId"].as_str().unwrap().contains(&"a".repeat(80)))
        .map(|task| dir.path().join(task["logFile"].as_str().unwrap()))
        .collect();
    assert_eq!(logs.len(), 2);
    assert_ne!(logs[0], logs[1]);
    let saved: Vec<_> = logs.iter().map(|path| fs::read(path).unwrap()).collect();
    for log in &logs {
        fs::remove_file(log).unwrap();
    }
    clear_executions(dir.path());
    succeeds(&execute(dir.path(), "check"));
    assert!(executions(dir.path()).is_empty());
    for (log, expected) in logs.iter().zip(saved) {
        assert_eq!(fs::read(log).unwrap(), expected);
    }
}

#[test]
fn ancestor_configuration_changes_invalidate_only_its_consumers() {
    let dir = fixture();
    succeeds(&execute(dir.path(), "check"));
    clear_executions(dir.path());
    write(
        dir.path(),
        "packages/app-a/tsconfig.json",
        "{\"compilerOptions\":{\"strict\":true}}",
    );
    succeeds(&execute(dir.path(), "check"));
    assert_eq!(executions(dir.path()), ["a"]);
}

#[cfg(unix)]
#[test]
fn symlinked_dependencies_fail_instead_of_guessing_runtime_resolution() {
    let dir = fixture();
    succeeds(&execute(dir.path(), "check"));
    clear_executions(dir.path());
    fs::remove_file(dir.path().join("packages/shared/alpha.mjs")).unwrap();
    std::os::unix::fs::symlink("beta.mjs", dir.path().join("packages/shared/alpha.mjs")).unwrap();
    let output = execute(dir.path(), "check");
    assert!(!output.status.success());
    assert!(
        combined_output(&output).contains("symlinked traced inputs"),
        "{}",
        combined_output(&output)
    );
    assert!(executions(dir.path()).is_empty());
}
