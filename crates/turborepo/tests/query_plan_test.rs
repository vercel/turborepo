#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{collections::HashSet, fs, path::Path};

use common::{run_turbo, run_turbo_with_env, setup};
use serde_json::{Value, json};

const PLAN_QUERY: &str = r#"query {
  globalEnvironment { env passThroughEnv }
  affectedTasks(base: "main", head: "HEAD", tasks: ["my-app#test", "another#test"]) {
    length
    items { fullName environment { env passThroughEnv } }
    withDependencies {
      length
      items {
        fullName
        name
        command
        package { name path }
        directDependencies { items { fullName } }
        environment { env passThroughEnv }
      }
    }
  }
}"#;

fn setup_plan(dir: &Path) {
    setup::copy_fixture("basic_monorepo", dir).unwrap();
    fs::write(
        dir.join("turbo.json"),
        serde_json::to_string_pretty(&json!({
            "futureFlags": {
                "experimentalTaskCommand": true,
                "affectedUsingTaskInputs": true
            },
            "globalEnv": ["GLOBAL_*", "!GLOBAL_SECRET"],
            "globalPassThroughEnv": ["CI_*"],
            "tasks": {
                "test": {
                    "command": ["echo", "test"],
                    "dependsOn": ["util#transit", "//#prepare"],
                    "env": ["TEST_*", "!TEST_SECRET"],
                    "passThroughEnv": ["TOKEN_*"]
                },
                "//#prepare": { "command": ["echo", "prepare"] },
                "util#transit": { "dependsOn": ["build"] },
                "util#build": { "dependsOn": ["leaf"], "env": ["BUILD_MODE"] },
                "util#leaf": {}
            }
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("apps/my-app/turbo.json"),
        serde_json::to_string_pretty(&json!({
            "extends": ["//"],
            "tasks": {
                "test": {
                    "env": ["$TURBO_EXTENDS$", "WEB_*"],
                    "passThroughEnv": ["$TURBO_EXTENDS$", "WEB_TOKEN"]
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("packages/another/turbo.json"),
        serde_json::to_string_pretty(&json!({
            "extends": ["//"],
            "tasks": { "test": { "env": ["API_*"], "passThroughEnv": [] } }
        }))
        .unwrap(),
    )
    .unwrap();
    setup::setup_git(dir).unwrap();
}

fn change_roots(dir: &Path) {
    fs::write(dir.join("apps/my-app/changed.ts"), "export {};").unwrap();
    fs::write(dir.join("packages/another/changed.ts"), "export {};").unwrap();
}

fn parse_query(output: std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "query failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json.get("errors").is_none(), "{json}");
    json
}

#[test]
fn test_query_plan_deduplicates_dependencies_and_preserves_all_edges() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_plan(tempdir.path());
    change_roots(tempdir.path());

    fs::write(tempdir.path().join("plan.gql"), PLAN_QUERY).unwrap();
    let json = parse_query(run_turbo(tempdir.path(), &["query", "plan.gql"]));
    let graph = &json["data"]["affectedTasks"]["withDependencies"];
    let items = graph["items"].as_array().unwrap();
    let names: Vec<_> = items
        .iter()
        .map(|item| item["fullName"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "//#prepare",
            "another#test",
            "my-app#test",
            "util#build",
            "util#leaf",
            "util#transit"
        ]
    );
    assert_eq!(graph["length"], names.len());
    let unique: HashSet<_> = names.iter().copied().collect();
    assert_eq!(unique.len(), names.len());
    for item in items {
        for dependency in item["directDependencies"]["items"].as_array().unwrap() {
            assert!(unique.contains(dependency["fullName"].as_str().unwrap()));
        }
    }
    for item in [&items[1], &items[2]] {
        assert_eq!(item["command"], "echo test");
        assert_eq!(
            item["directDependencies"]["items"],
            json!([{ "fullName": "//#prepare" }, { "fullName": "util#transit" }])
        );
    }
    assert_eq!(items[0]["command"], "echo prepare");
    assert_eq!(items[0]["package"], json!({ "name": "//", "path": "" }));
    assert_eq!(items[2]["package"]["path"], "apps/my-app");
    assert_eq!(
        items[3]["directDependencies"]["items"],
        json!([{ "fullName": "util#leaf" }])
    );
    assert!(items[4]["command"].is_null());
    assert_eq!(items[4]["directDependencies"]["items"], json!([]));
    assert!(items[5]["command"].is_null());
    assert_eq!(
        items[5]["directDependencies"]["items"],
        json!([{ "fullName": "util#build" }])
    );

    // The existing affected-task list still omits non-executable leaf tasks.
    let affected = json["data"]["affectedTasks"]["items"].as_array().unwrap();
    assert!(!affected.iter().any(|item| item["fullName"] == "util#leaf"));
}

#[test]
fn test_query_plan_exposes_resolved_patterns_without_environment_values() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_plan(tempdir.path());
    change_roots(tempdir.path());

    let json = parse_query(run_turbo_with_env(
        tempdir.path(),
        &["query", PLAN_QUERY],
        &[
            ("GLOBAL_EXAMPLE", "secret-global-value"),
            ("WEB_TOKEN", "secret-token-value"),
        ],
    ));
    assert_eq!(
        json["data"]["globalEnvironment"],
        json!({ "env": ["!GLOBAL_SECRET", "GLOBAL_*"], "passThroughEnv": ["CI_*"] })
    );
    let items = json["data"]["affectedTasks"]["withDependencies"]["items"]
        .as_array()
        .unwrap();
    for (name, expected) in [
        (
            "my-app#test",
            json!({ "env": ["!TEST_SECRET", "TEST_*", "WEB_*"], "passThroughEnv": ["TOKEN_*", "WEB_TOKEN"] }),
        ),
        (
            "another#test",
            json!({ "env": ["API_*"], "passThroughEnv": [] }),
        ),
        (
            "util#build",
            json!({ "env": ["BUILD_MODE"], "passThroughEnv": [] }),
        ),
        ("util#leaf", json!({ "env": [], "passThroughEnv": [] })),
    ] {
        let task = items.iter().find(|item| item["fullName"] == name).unwrap();
        assert_eq!(task["environment"], expected, "{name}");
        if let Some(affected) = json["data"]["affectedTasks"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["fullName"] == name)
        {
            assert_eq!(affected["environment"], expected, "{name}");
        }
    }
    let serialized = json.to_string();
    assert!(!serialized.contains("secret-global-value"));
    assert!(!serialized.contains("secret-token-value"));
    assert!(!serialized.contains("$TURBO_EXTENDS$"));
}

#[test]
fn test_query_plan_matches_dry_run_graph_and_environment_patterns() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_plan(tempdir.path());
    change_roots(tempdir.path());
    let plan = parse_query(run_turbo(tempdir.path(), &["query", PLAN_QUERY]));
    let dry = parse_query(run_turbo(
        tempdir.path(),
        &[
            "run",
            "my-app#test",
            "another#test",
            "--dry=json",
            "--cache=local:,remote:",
        ],
    ));
    let nodes = plan["data"]["affectedTasks"]["withDependencies"]["items"]
        .as_array()
        .unwrap();
    let dry_tasks = dry["tasks"].as_array().unwrap();
    assert_eq!(nodes.len(), dry_tasks.len());
    for task in dry_tasks {
        let node = nodes
            .iter()
            .find(|node| node["fullName"] == task["taskId"])
            .unwrap();
        let dependencies: HashSet<_> = node["directDependencies"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["fullName"].as_str().unwrap())
            .collect();
        let dry_dependencies: HashSet<_> = task["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item.as_str().unwrap())
            .collect();
        assert_eq!(dependencies, dry_dependencies);
        for field in ["env", "passThroughEnv"] {
            let expected = task["environmentVariables"]["specified"][field]
                .as_array()
                .cloned()
                .unwrap_or_default();
            assert_eq!(node["environment"][field], json!(expected));
        }
    }
    for field in ["env", "passThroughEnv"] {
        let expected = dry["globalCacheInputs"]["environmentVariables"]["specified"][field]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert_eq!(plan["data"]["globalEnvironment"][field], json!(expected));
    }
}

#[test]
fn test_query_plan_shared_chain_is_emitted_once_in_stable_order() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::copy_fixture("basic_monorepo", tempdir.path()).unwrap();
    let mut tasks = serde_json::Map::new();
    let roots: Vec<_> = (0..32).map(|i| format!("my-app#check-{i:02}")).collect();
    for root in &roots {
        tasks.insert(
            root.clone(),
            json!({
                "command": ["echo", "check"],
                "dependsOn": ["util#stage-00"]
            }),
        );
    }
    for i in 0..64 {
        let dependencies = if i < 63 {
            vec![format!("util#stage-{:02}", i + 1)]
        } else {
            Vec::new()
        };
        tasks.insert(
            format!("util#stage-{i:02}"),
            json!({ "dependsOn": dependencies }),
        );
    }
    fs::write(
        tempdir.path().join("turbo.json"),
        serde_json::to_string(&json!({
            "futureFlags": { "experimentalTaskCommand": true, "affectedUsingTaskInputs": true },
            "tasks": tasks
        }))
        .unwrap(),
    )
    .unwrap();
    setup::setup_git(tempdir.path()).unwrap();
    fs::write(tempdir.path().join("apps/my-app/changed.ts"), "export {};").unwrap();
    let query = format!(
        r#"query {{
      affectedTasks(base: "main", head: "HEAD", tasks: {}) {{
        withDependencies {{ length items {{ fullName directDependencies {{ items {{ fullName }} }} }} }}
      }}
    }}"#,
        serde_json::to_string(&roots).unwrap()
    );
    let plan = parse_query(run_turbo(tempdir.path(), &["query", &query]));
    let graph = &plan["data"]["affectedTasks"]["withDependencies"];
    assert_eq!(graph["length"], 96);
    let items = graph["items"].as_array().unwrap();
    let names: Vec<_> = items
        .iter()
        .map(|item| item["fullName"].as_str().unwrap())
        .collect();
    assert_eq!(names.iter().copied().collect::<HashSet<_>>().len(), 96);
    assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
    let edges: usize = items
        .iter()
        .map(|item| {
            item["directDependencies"]["items"]
                .as_array()
                .unwrap()
                .len()
        })
        .sum();
    assert_eq!(edges, 95);
    assert_eq!(
        plan,
        parse_query(run_turbo(tempdir.path(), &["query", &query]))
    );
}

#[test]
fn test_query_plan_empty_selection() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_plan(tempdir.path());
    let json = parse_query(run_turbo(tempdir.path(), &["query", PLAN_QUERY]));
    assert_eq!(
        json["data"]["affectedTasks"],
        json!({ "length": 0, "items": [], "withDependencies": { "length": 0, "items": [] } })
    );

    change_roots(tempdir.path());
    let query = PLAN_QUERY
        .replace("my-app#test", "nonexistent")
        .replace("another#test", "nonexistent");
    let json = parse_query(run_turbo(tempdir.path(), &["query", &query]));
    assert_eq!(
        json["data"]["affectedTasks"]["withDependencies"]["length"],
        0
    );
}

#[test]
fn test_query_environment_fields_work_without_affected_selection() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::copy_fixture("basic_monorepo", tempdir.path()).unwrap();
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{ "tasks": { "build": {} } }"#,
    )
    .unwrap();
    let json = parse_query(run_turbo(
        tempdir.path(),
        &[
            "query",
            r#"query {
          globalEnvironment { env passThroughEnv }
          package(name: "util") {
            tasks { items { fullName environment { env passThroughEnv } } }
          }
        }"#,
        ],
    ));
    let empty = json!({ "env": [], "passThroughEnv": [] });
    assert_eq!(json["data"]["globalEnvironment"], empty);
    for task in json["data"]["package"]["tasks"]["items"]
        .as_array()
        .unwrap()
    {
        assert_eq!(task["environment"], empty);
    }
}
