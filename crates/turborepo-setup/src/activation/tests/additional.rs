//! Repository-declared additional tools join the same activation plan.
use super::*;
use crate::artifact_provision::{self, ArtifactPlan};

fn declare(repo: &Path, tools: Value) {
    let config = json!({"futureFlags": {"experimentalSetup": true},
        "setup": {"additionalTools": tools}});
    fs::write(repo.join("turbo.json"), config.to_string()).unwrap();
}

fn tool(platform: &str, executable: &str) -> Value {
    json!({"version": "1.2.3", "artifacts": {platform: {
        "url": "https://downloads.example.test/fixture-1.2.3.tar.gz",
        "sha256": "66".repeat(32), "format": "tar-gz", "rootPrefix": "fixture-1.2.3",
        "executables": {executable: "bin/fixture"}}}})
}

/// The Node/pnpm lock plus the projection of the root declarations.
fn locked(repo: &Path) -> Value {
    let mut value = fixture();
    write_sources(repo, &value);
    let projected = Snapshot::capture(repo).unwrap().additional_tools().clone();
    for (id, tool) in projected {
        value["tools"][id] = serde_json::to_value(tool).unwrap();
    }
    fs::write(repo.join("turbo.lock"), value.to_string()).unwrap();
    value
}

fn with_artifacts(value: &Value) -> Vec<Tool> {
    let lock = Lock::parse(value.to_string().as_bytes()).unwrap();
    let mut tools = desired(value, None);
    let plans = ArtifactPlan::from_lock(&lock, Platform::MacosArm64).unwrap();
    tools.extend(plans.iter().map(|plan| plan.inventory_tool().clone()));
    tools.sort_by(|a, b| a.id.cmp(&b.id));
    tools
}

#[test]
fn declared_executables_share_the_managed_bin_and_removal_is_drift() {
    let repo = tempfile::tempdir().unwrap();
    declare(
        repo.path(),
        json!({"fixture": tool("macos-arm64", "fixture")}),
    );
    let value = locked(repo.path());
    let plan = inspect(repo.path(), context()).unwrap();
    assert_eq!(plan.tools(), with_artifacts(&value));
    assert_eq!(plan.snapshot().tools().len(), 3);
    let installed = activate(repo.path(), &plan);
    assert!(installed.vars().is_empty());
    // The export resolves into the full tree, beside its adjacent resources.
    let target = fs::canonicalize(installed.bin().join("fixture")).unwrap();
    let tree = target.parent().unwrap().parent().unwrap();
    assert!(tree.join("resources/template.json").is_file());

    // A selection without the tool is another environment.
    let without = turborepo_tool_install::Selection::new(desired(&value, None)).unwrap();
    assert_ne!(without.key(), plan.selection().key());
    // Removing or editing the declaration is drift with the old lock.
    for tools in [
        json!({}),
        json!({"fixture": tool("macos-arm64", "fixture2")}),
    ] {
        declare(repo.path(), tools);
        assert!(matches!(
            inspect(repo.path(), context()),
            Err(Error::DeclarationDrift)
        ));
    }
}

#[test]
fn a_tool_without_an_artifact_for_this_platform_never_activates() {
    let repo = tempfile::tempdir().unwrap();
    declare(
        repo.path(),
        json!({"fixture": tool("linux-x64-gnu", "fixture")}),
    );
    locked(repo.path());
    let error = inspect(repo.path(), context()).err().unwrap();
    assert!(
        matches!(&error, Error::AdditionalTool(artifact_provision::Error::MissingTarget {
            tool, platform
        }) if tool == "fixture" && platform == "macos-arm64"),
        "{error}"
    );
}

#[test]
fn exports_that_would_shadow_another_tool_are_rejected() {
    let tool = |id: &str, platform: &str, names: &[&str]| Tool {
        id: id.into(),
        version: "1.0.0".into(),
        platform: platform.into(),
        artifact_sha256: "77".repeat(32),
        executables: names
            .iter()
            .map(|name| (name.to_string(), format!("bin/{name}").into()))
            .collect(),
        vars: Default::default(),
    };
    let node = tool("node", "macos-arm64", &["node", "npm", "npx"]);
    let check = |additional: &[Tool]| artifact_provision::check_exports([&node], additional);
    assert!(check(&[tool("protoc", "macos-arm64", &["protoc"])]).is_ok());
    for (additional, executable, other) in [
        (
            vec![tool("fixture", "macos-arm64", &["Node"])],
            "Node",
            "node",
        ),
        (
            vec![
                tool("alpha", "macos-arm64", &["jq"]),
                tool("beta", "macos-arm64", &["JQ"]),
            ],
            "JQ",
            "alpha",
        ),
        (
            vec![
                tool("alpha", "windows-x64", &["jq"]),
                tool("beta", "windows-x64", &["jq.exe"]),
            ],
            "jq.exe",
            "alpha",
        ),
    ] {
        let error = check(&additional).unwrap_err();
        assert!(
            matches!(&error, artifact_provision::Error::Collision {
                executable: name, other: owner, ..
            } if name == executable && owner == other),
            "{error}"
        );
    }
    // Only Windows resolves `jq.exe` as `jq`.
    assert!(
        check(&[
            tool("alpha", "linux-x64-gnu", &["jq"]),
            tool("beta", "linux-x64-gnu", &["jq.exe"]),
        ])
        .is_ok()
    );
}
