mod common;

use std::fs;

use common::{git, run_turbo, run_turbo_with_env, setup};

fn setup_affected(dir: &std::path::Path) {
    setup::setup_integration_test(dir, "basic_monorepo", "npm@10.5.0", true).unwrap();
    git(dir, &["checkout", "-b", "my-branch"]);
}

#[test]
fn test_nothing_affected_on_new_branch() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    let output = run_turbo(tempdir.path(), &["ls", "--affected"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0 no packages"),
        "nothing should be affected: {stdout}"
    );
}

#[test]
fn test_affected_run_with_file_change() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--affected", "--log-order", "grouped"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Packages in scope: my-app"));
    assert!(stdout.contains("1 successful, 1 total"));
}

#[test]
fn test_affected_ls_with_file_change() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();

    let output = run_turbo(tempdir.path(), &["ls", "--affected"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 package"));
    assert!(stdout.contains("my-app"));
}

#[test]
fn test_affected_tasks_dependency_propagation() {
    let tempdir = tempfile::tempdir().unwrap();
    // Use the task_dependencies/query fixture which has ^build0 dependencies
    setup::setup_integration_test(
        tempdir.path(),
        "task_dependencies/query",
        "npm@10.5.0",
        true,
    )
    .unwrap();
    git(tempdir.path(), &["checkout", "-b", "my-branch"]);

    // lib-a has no dependents with ^build0 in this fixture, but app-a depends
    // on lib-a and has build0 dependsOn: ['^build0']. Changing a file in lib-a
    // should propagate to app-a's build0 task through the task graph.
    fs::write(tempdir.path().join("lib-a/new.js"), "hello world").unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks { items { name package { name } reason { __typename } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedTasks"]["items"].as_array().unwrap();

    // lib-a's tasks should be directly affected
    let lib_a_tasks: Vec<&str> = items
        .iter()
        .filter(|i| i["package"]["name"] == "lib-a")
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert!(
        !lib_a_tasks.is_empty(),
        "lib-a should have affected tasks: {items:?}"
    );

    // app-a depends on lib-a and has:
    //   test dependsOn: ['^build0', 'prepare']
    // So app-a#test should be affected via task dependency propagation from
    // lib-a#build0
    let app_a_tasks: Vec<&str> = items
        .iter()
        .filter(|i| i["package"]["name"] == "app-a")
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert!(
        app_a_tasks.contains(&"test"),
        "app-a#test should be affected through ^build0 dependency on lib-a: {items:?}"
    );

    // Tasks propagated through the task graph should have TaskDependencyTaskChanged
    // reason
    let app_a_test = items
        .iter()
        .find(|i| i["package"]["name"] == "app-a" && i["name"] == "test")
        .unwrap();
    assert_eq!(
        app_a_test["reason"]["__typename"], "TaskDependencyTaskChanged",
        "app-a#test reason should be TaskDependencyTaskChanged: {app_a_test:?}"
    );
}

#[test]
fn test_affected_query_file_changed() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedPackages { items { name reason { __typename } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedPackages"]["items"]
        .as_array()
        .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "my-app");
    assert_eq!(items[0]["reason"]["__typename"], "FileChanged");
}

#[test]
fn test_affected_dependency_changed() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("packages/util/new.js"), "hello world").unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedPackages { items { name reason { __typename } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedPackages"]["items"]
        .as_array()
        .unwrap();
    let names: Vec<&str> = items.iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"util"));
    assert!(names.contains(&"my-app"));

    let util_item = items.iter().find(|i| i["name"] == "util").unwrap();
    assert_eq!(util_item["reason"]["__typename"], "FileChanged");
    let app_item = items.iter().find(|i| i["name"] == "my-app").unwrap();
    assert_eq!(app_item["reason"]["__typename"], "DependencyChanged");
}

#[test]
fn test_affected_committed_change() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    let pkg_path = tempdir.path().join("apps/my-app/package.json");
    let contents = fs::read_to_string(&pkg_path).unwrap();
    let mut pkg: serde_json::Value = serde_json::from_str(&contents).unwrap();
    pkg["description"] = serde_json::Value::String("foo".to_string());
    fs::write(&pkg_path, serde_json::to_string_pretty(&pkg).unwrap()).unwrap();

    git(tempdir.path(), &["add", "."]);
    git(tempdir.path(), &["commit", "-m", "add foo", "--quiet"]);

    let output = run_turbo(tempdir.path(), &["ls", "--affected"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("my-app"));
}

#[test]
fn test_affected_scm_base_override() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();
    git(tempdir.path(), &["add", "."]);
    git(tempdir.path(), &["commit", "-m", "add foo", "--quiet"]);

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--affected", "--log-order", "grouped"],
        &[("TURBO_SCM_BASE", "HEAD")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0 successful, 0 total"),
        "SCM_BASE=HEAD should show no changes: {stdout}"
    );
}

#[test]
fn test_affected_scm_head_override() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();
    git(tempdir.path(), &["add", "."]);
    git(tempdir.path(), &["commit", "-m", "add foo", "--quiet"]);

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--affected", "--log-order", "grouped"],
        &[("TURBO_SCM_HEAD", "main")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0 successful, 0 total"),
        "SCM_HEAD=main should show no changes: {stdout}"
    );
}

#[test]
fn test_affected_merge_base_diverged() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    let pkg_path = tempdir.path().join("apps/my-app/package.json");
    let contents = fs::read_to_string(&pkg_path).unwrap();
    let mut pkg: serde_json::Value = serde_json::from_str(&contents).unwrap();
    pkg["description"] = serde_json::Value::String("foo".to_string());
    fs::write(&pkg_path, serde_json::to_string_pretty(&pkg).unwrap()).unwrap();
    git(tempdir.path(), &["add", "."]);
    git(tempdir.path(), &["commit", "-m", "add foo", "--quiet"]);

    git(tempdir.path(), &["checkout", "main", "--quiet"]);
    let index_path = tempdir.path().join("packages/util/index.js");
    let mut idx = fs::read_to_string(&index_path).unwrap_or_default();
    idx.push_str("\nfoo");
    fs::write(&index_path, idx).unwrap();
    git(tempdir.path(), &["add", "."]);
    git(tempdir.path(), &["commit", "-m", "add foo", "--quiet"]);
    git(tempdir.path(), &["checkout", "my-branch", "--quiet"]);

    let output = run_turbo(tempdir.path(), &["ls", "--affected"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("my-app"));
    assert!(
        !stdout.contains("util"),
        "util changed on main, not branch: {stdout}"
    );
}

// ── affectedTasks tests ──

#[test]
fn test_affected_tasks_nothing_on_new_branch() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks { length items { name fullName reason { __typename } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["data"]["affectedTasks"]["length"], 0);
}

#[test]
fn test_affected_tasks_file_change() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks { items { name fullName package { name } reason { __typename } \
             } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedTasks"]["items"].as_array().unwrap();

    // my-app has build and maybefails scripts — both should be affected
    let my_app_tasks: Vec<&str> = items
        .iter()
        .filter(|i| i["package"]["name"] == "my-app")
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert!(
        my_app_tasks.contains(&"build"),
        "my-app#build should be affected: {items:?}"
    );
    assert!(
        my_app_tasks.contains(&"maybefails"),
        "my-app#maybefails should be affected: {items:?}"
    );

    // All my-app tasks should have TaskFileChanged reason
    for item in items.iter().filter(|i| i["package"]["name"] == "my-app") {
        assert_eq!(
            item["reason"]["__typename"], "TaskFileChanged",
            "expected TaskFileChanged for my-app task: {item:?}"
        );
    }
}

#[test]
fn test_affected_tasks_input_exclusion() {
    let tempdir = tempfile::tempdir().unwrap();
    // Use a special fixture that has task input exclusions
    setup::setup_integration_test(tempdir.path(), "affected_tasks_inputs", "npm@10.5.0", true)
        .unwrap();
    git(tempdir.path(), &["checkout", "-b", "my-branch"]);

    // Change a .md file which is excluded from the test task's inputs
    fs::write(
        tempdir.path().join("packages/lib-a/README.md"),
        "updated docs",
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks { items { name package { name } reason { __typename } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedTasks"]["items"].as_array().unwrap();

    // The "test" task excludes *.md files, so it should NOT be affected
    let test_tasks: Vec<_> = items
        .iter()
        .filter(|i| i["package"]["name"] == "lib-a" && i["name"] == "test")
        .collect();
    assert!(
        test_tasks.is_empty(),
        "lib-a#test should NOT be affected by .md changes (excluded by inputs): {items:?}"
    );

    // The "build" task uses $TURBO_DEFAULT$ without exclusions, so it SHOULD be
    // affected
    let build_tasks: Vec<_> = items
        .iter()
        .filter(|i| i["package"]["name"] == "lib-a" && i["name"] == "build")
        .collect();
    assert!(
        !build_tasks.is_empty(),
        "lib-a#build SHOULD be affected by .md changes (default inputs): {items:?}"
    );
}

#[test]
fn test_affected_tasks_global_dep_change() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    // Change foo.txt which is a globalDependency
    fs::write(tempdir.path().join("foo.txt"), "changed").unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks { length items { name package { name } reason { __typename } } \
             } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let length = json["data"]["affectedTasks"]["length"].as_i64().unwrap();
    // All tasks should be affected when a globalDependency changes
    assert!(
        length > 0,
        "all tasks should be affected when globalDependency changes"
    );

    let items = json["data"]["affectedTasks"]["items"].as_array().unwrap();
    // All should have GlobalDepsChanged reason
    for item in items {
        assert_eq!(
            item["reason"]["__typename"], "TaskGlobalDepsChanged",
            "expected TaskGlobalDepsChanged reason: {item:?}"
        );
    }
}
