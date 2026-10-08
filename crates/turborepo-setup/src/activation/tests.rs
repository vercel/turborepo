#![allow(clippy::unwrap_used)]

use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

use serde_json::{Value, json};
use turborepo_platform::{Architecture, OperatingSystem, Platform as HostPlatform};

use super::*;
use crate::{execution_identity::Libc, lock::Lock};

fn context() -> ExecutionContext {
    ExecutionContext::new(
        HostPlatform::new(OperatingSystem::Macos, Architecture::Arm64),
        Libc::None,
        "aarch64-apple-darwin".into(),
        BTreeSet::from(["baseline".into()]),
    )
    .unwrap()
}

fn fixture() -> Value {
    json!({"schemaVersion":0,"tools":{
        "node":{"adapter":"node","version":"24.0.0",
            "options":{"bundled-npm":["11.0.0"]},
            "declarations":[{"file":".nvmrc","request":"24.0.0"}],
            "installation":{"kind":"managed","artifacts":{"macos-arm64":{"distribution":{
                "url":"https://nodejs.org/dist/v24.0.0/node-v24.0.0-darwin-arm64.tar.gz",
                "sha256":"11".repeat(32),"format":"tar-gz","rootPrefix":"node-v24.0.0-darwin-arm64",
                "executables":{"node":"bin/node","npm":"bin/npm","npx":"bin/npx"}
            }}}}},
        "pnpm":{"adapter":"pnpm","version":"10.0.0",
            "declarations":[{"file":"package.json","field":"/packageManager","request":"pnpm@10.0.0"}],
            "installation":{"kind":"managed","artifacts":{"any":{"package":{
                "url":"https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz",
                "sha256":"22".repeat(32),"format":"tar-gz","rootPrefix":"package",
                "executables":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}
            }}}}}
    }})
}

fn write_sources(repo: &Path, value: &Value) {
    fs::write(repo.join("turbo.lock"), value.to_string()).unwrap();
    fs::write(repo.join(".nvmrc"), "24.0.0\n").unwrap();
    fs::write(
        repo.join("package.json"),
        r#"{"packageManager":"pnpm@10.0.0"}"#,
    )
    .unwrap();
}

fn desired(
    value: &Value,
    authored: Option<&crate::package_manager::CorepackIntegrity>,
) -> Vec<Tool> {
    let lock = Lock::parse(value.to_string().as_bytes()).unwrap();
    let node = NodePlan::from_lock(&lock, Platform::MacosArm64).unwrap();
    let mut tools = vec![node.inventory_tool().clone()];
    if lock.tools().contains_key("pnpm") {
        tools.push(
            PnpmPlan::from_lock(&lock, Platform::MacosArm64, &node, authored)
                .unwrap()
                .inventory_tool()
                .clone(),
        );
    }
    tools
}

// Readiness fixtures only, not executable authorization or vendor verification.
// Every export is a trap: any probe would fail and leave an observable marker.
fn stage(tool: &Tool, root: &Path) -> Result<(), turborepo_tool_install::Error> {
    for path in tool.executables.values() {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, "#!/bin/sh\ntouch activation-was-probed\nexit 99\n")?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    fs::create_dir_all(root.join("resources"))?;
    fs::write(
        root.join("resources/template.json"),
        "adjacent runtime resource",
    )?;
    Ok(())
}

fn seed(repo: &Path, tools: &[Tool]) -> PathBuf {
    let mut store = Store::open(repo).unwrap();
    store.reconcile(tools, stage).unwrap();
    store.current().unwrap().unwrap().bin
}

fn installed() -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    write_sources(repo.path(), &fixture());
    seed(repo.path(), &desired(&fixture(), None));
    repo
}

fn state(root: &Path) -> Vec<(PathBuf, Vec<u8>, u32, std::time::SystemTime)> {
    fn walk(
        root: &Path,
        dir: &Path,
        out: &mut Vec<(PathBuf, Vec<u8>, u32, std::time::SystemTime)>,
    ) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            let metadata = fs::symlink_metadata(&path).unwrap();
            let bytes = if metadata.file_type().is_symlink() {
                fs::read_link(&path)
                    .unwrap()
                    .as_os_str()
                    .as_encoded_bytes()
                    .to_vec()
            } else if metadata.is_file() {
                fs::read(&path).unwrap()
            } else {
                Vec::new()
            };
            out.push((
                path.strip_prefix(root).unwrap().into(),
                bytes,
                metadata.permissions().mode(),
                metadata.modified().unwrap(),
            ));
            if metadata.is_dir() {
                walk(root, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn complete_plan_is_read_only_and_preserves_caller_path() {
    let repo = installed();
    // Inspection must not need/recreate the setup transaction lock.
    fs::remove_file(repo.path().join(".turbo/tools/transaction.lock")).unwrap();
    let before = state(repo.path());
    let env: BTreeMap<_, _> = std::env::vars_os().collect();
    let plan = ActivationPlan::inspect(repo.path(), context()).unwrap();
    assert_eq!(plan.tools(), desired(&fixture(), None));
    assert_eq!(plan.snapshot().tools().len(), 2);
    assert!(plan.runtime_env().is_empty());
    let inherited = OsStr::new("/host/bin::/other bin");
    let paths: Vec<_> = std::env::split_paths(&plan.path(Some(inherited)).unwrap()).collect();
    assert_eq!(paths[0], plan.path_prepend());
    assert_eq!(
        &paths[1..],
        &[
            PathBuf::from("/host/bin"),
            PathBuf::new(),
            PathBuf::from("/other bin")
        ]
    );
    assert_eq!(plan.path(None).unwrap(), plan.path_prepend().as_os_str());
    assert_eq!(state(repo.path()), before);
    assert_eq!(std::env::vars_os().collect::<BTreeMap<_, _>>(), env);
}

#[test]
fn compatible_relocation_keeps_snapshot_not_absolute_install_paths() {
    let repo = installed();
    let before = ActivationPlan::inspect(repo.path(), context()).unwrap();
    let relocated = tempfile::tempdir().unwrap();
    let target = relocated.path().join("repo ' relocated");
    fs::rename(repo.path(), &target).unwrap();
    let after = ActivationPlan::inspect(&target, context()).unwrap();
    assert_ne!(before.path_prepend(), after.path_prepend());
    assert!(
        after
            .path_prepend()
            .starts_with(target.canonicalize().unwrap())
    );
    assert_eq!(
        before.snapshot().canonical_identity(),
        after.snapshot().canonical_identity()
    );
    assert_eq!(
        before.snapshot().fingerprint(),
        after.snapshot().fingerprint()
    );
}

#[test]
fn missing_lock_and_inventory_never_create_storage_or_use_ancestors() {
    let parent = installed();
    let repo = parent.path().join("nested");
    fs::create_dir(&repo).unwrap();
    let before = state(&repo);
    assert!(matches!(
        ActivationPlan::inspect(&repo, context()),
        Err(Error::MissingLock)
    ));
    assert_eq!(state(&repo), before);
    write_sources(&repo, &fixture());
    let before = state(&repo);
    assert!(matches!(
        ActivationPlan::inspect(&repo, context()),
        Err(Error::MissingInventory)
    ));
    assert_eq!(state(&repo), before);
    assert!(!repo.join(".turbo").exists());
}

#[test]
fn stale_or_unrelated_inventory_never_supplies_extra_exports() {
    for change in 0..4 {
        let repo = tempfile::tempdir().unwrap();
        write_sources(repo.path(), &fixture());
        let mut tools = desired(&fixture(), None);
        match change {
            0 => {
                tools.pop();
            }
            1 => tools[0].artifact_sha256 = "33".repeat(32),
            2 => tools[0].platform = "darwin-x64".into(),
            _ => tools.push(Tool {
                id: "unrelated".into(),
                version: "1".into(),
                platform: "any".into(),
                artifact_sha256: "44".repeat(32),
                executables: BTreeMap::from([("foreign".into(), "bin/tool".into())]),
            }),
        }
        seed(repo.path(), &tools);
        let before = state(repo.path());
        assert!(matches!(
            ActivationPlan::inspect(repo.path(), context()),
            Err(Error::StaleInventory)
        ));
        assert_eq!(state(repo.path()), before);
    }
}

#[test]
fn every_export_and_adjacent_resource_is_checked() {
    for name in ["node", "npm", "npx", "pnpm", "pnpx", "resource", "extra"] {
        let repo = installed();
        let bin = Store::inspect(repo.path()).unwrap().unwrap().bin;
        match name {
            "resource" => {
                fs::write(bin.join("../tools/pnpm/resources/template.json"), "damaged").unwrap()
            }
            "extra" => {
                symlink("../tools/node/bin/node", bin.join("undeclared")).unwrap();
            }
            _ => fs::remove_file(bin.join(name)).unwrap(),
        }
        let before = state(repo.path());
        assert!(
            matches!(
                ActivationPlan::inspect(repo.path(), context()),
                Err(Error::DamagedInventory(_))
            ),
            "{name}"
        );
        assert_eq!(state(repo.path()), before);
    }
}

#[test]
fn unsafe_managed_paths_and_escaped_exports_fail_closed() {
    for change in 0..4 {
        let repo = installed();
        let bin = Store::inspect(repo.path()).unwrap().unwrap().bin;
        let outside = tempfile::tempdir().unwrap();
        match change {
            0 => {
                fs::remove_file(bin.join("pnpx")).unwrap();
                symlink(outside.path(), bin.join("pnpx")).unwrap();
            }
            1 => {
                fs::rename(repo.path().join(".turbo"), outside.path().join("tools")).unwrap();
                symlink(outside.path().join("tools"), repo.path().join(".turbo")).unwrap();
            }
            2 => fs::set_permissions(bin.parent().unwrap(), fs::Permissions::from_mode(0o777))
                .unwrap(),
            _ => fs::set_permissions(bin.join("node"), fs::Permissions::from_mode(0o644)).unwrap(),
        }
        let before = state(repo.path());
        assert!(matches!(
            ActivationPlan::inspect(repo.path(), context()),
            Err(Error::DamagedInventory(_))
        ));
        assert_eq!(state(repo.path()), before);
    }
}

#[test]
fn native_provenance_and_capture_changes_are_rejected() {
    let repo = installed();
    let captured = Snapshot::capture(repo.path()).unwrap();
    fs::write(repo.path().join(".nvmrc"), "24.x").unwrap();
    let before = state(repo.path());
    assert!(matches!(
        ActivationPlan::inspect(repo.path(), context()),
        Err(Error::DeclarationDrift)
    ));
    assert!(matches!(
        ActivationPlan::from_sources(repo.path(), &captured, context()),
        Err(Error::Sources(StorageError::Conflict))
    ));
    assert_eq!(state(repo.path()), before);
}

#[test]
fn authored_integrity_is_part_of_the_installed_node_binding() {
    let version = format!("10.0.0+sha512.{}", "aa".repeat(64));
    for manifest in [
        json!({"packageManager":format!("pnpm@{version}")}),
        json!({"devEngines":{"packageManager":{"name":"pnpm","version":version}}}),
        // The top-level version does not erase an applicable devEngines digest.
        json!({"packageManager":"pnpm@10.0.0","devEngines":{"packageManager":{"name":"pnpm","version":version}}}),
        // Both digests apply; SHA-256 must match the lock and SHA-512 binds reuse.
        json!({"packageManager":format!("pnpm@10.0.0+sha256.{}", "22".repeat(32)),
            "devEngines":{"packageManager":{"name":"pnpm","version":version}}}),
        json!({"packageManager":format!("pnpm@{version}"),
            "devEngines":{"packageManager":{"name":"pnpm","version":format!("10.0.0+sha256.{}", "22".repeat(32))}}}),
    ] {
        let repo = installed();
        fs::write(repo.path().join("package.json"), manifest.to_string()).unwrap();
        let captured = Snapshot::capture(repo.path()).unwrap();
        let mut value = fixture();
        value["tools"]["pnpm"]["declarations"] =
            serde_json::to_value(&captured.declarations()["pnpm"]).unwrap();
        fs::write(repo.path().join("turbo.lock"), value.to_string()).unwrap();
        assert!(matches!(
            ActivationPlan::inspect(repo.path(), context()),
            Err(Error::StaleInventory)
        ));
        let authored = crate::package_manager::CorepackIntegrity {
            algorithm: "sha512",
            digest: "aa".repeat(64),
        };
        seed(repo.path(), &desired(&value, Some(&authored)));
        assert!(ActivationPlan::inspect(repo.path(), context()).is_ok());
    }
}

#[test]
fn authored_integrity_errors_precede_inventory_reads_without_mutation() {
    let wrong = format!("10.0.0+sha256.{}", "ff".repeat(32));
    let weak = format!("10.0.0+sha1.{}", "aa".repeat(20));
    let strong = format!("10.0.0+sha512.{}", "aa".repeat(64));
    let other = format!("10.0.0+sha512.{}", "bb".repeat(64));
    let dev = |version: &str| json!({"name":"pnpm","version":version});
    for manifest in [
        json!({"packageManager":format!("pnpm@{wrong}")}),
        json!({"packageManager":"pnpm@10.0.0","devEngines":{"packageManager":dev(&wrong)}}),
        json!({"packageManager":"pnpm@10.0.0","devEngines":{"packageManager":dev(&weak)}}),
        // Discovery permits an advisory conflict, but applicable pins still bind.
        json!({"packageManager":format!("pnpm@{strong}"),"devEngines":{"packageManager":{
            "name":"pnpm","version":other,"onFail":"warn"}}}),
        json!({"devEngines":{"packageManager":[dev(&strong),dev("10.x")]}}),
        json!({"devEngines":{"packageManager":[dev(&strong),dev(&other)]}}),
    ] {
        let repo = tempfile::tempdir().unwrap();
        write_sources(repo.path(), &fixture());
        fs::write(repo.path().join("package.json"), manifest.to_string()).unwrap();
        let captured = Snapshot::capture(repo.path()).unwrap();
        let mut value = fixture();
        value["tools"]["pnpm"]["declarations"] =
            serde_json::to_value(&captured.declarations()["pnpm"]).unwrap();
        fs::write(repo.path().join("turbo.lock"), value.to_string()).unwrap();
        let before = state(repo.path());
        assert!(
            matches!(
                ActivationPlan::inspect(repo.path(), context()),
                Err(Error::InvalidLock)
            ),
            "{manifest}"
        );
        assert_eq!(state(repo.path()), before);
        assert!(!repo.path().join(".turbo").exists());
    }
}

#[test]
fn unsupported_adapters_platforms_and_invalid_lock_metadata_are_distinct() {
    let repo = installed();
    for (host, libc) in [
        (
            HostPlatform::new(OperatingSystem::Windows, Architecture::Arm64),
            Libc::None,
        ),
        (
            HostPlatform::new(OperatingSystem::Linux, Architecture::X64),
            Libc::Musl { abi: "1.2".into() },
        ),
    ] {
        let context = ExecutionContext::new(
            host,
            libc,
            "target".into(),
            BTreeSet::from(["baseline".into()]),
        )
        .unwrap();
        assert!(matches!(
            ActivationPlan::inspect(repo.path(), context),
            Err(Error::UnsupportedPlatform)
        ));
    }
    for change in 0..4 {
        let mut value = fixture();
        match change {
            0 => {
                value["tools"]["node"]["installation"] =
                    json!({"kind":"verify-system","executables":["node"]})
            }
            1 => {
                value["tools"]["npm"] = value["tools"]["pnpm"].clone();
                value["tools"]["npm"]["adapter"] = json!("npm");
                value["tools"]["npm"]["installation"] =
                    json!({"kind":"verify-system","executables":["independent-npm"]});
            }
            2 => value["tools"]["pnpm"]["adapter"] = json!("future"),
            _ => {
                value["tools"]["node"]["installation"]["artifacts"]["macos-arm64"]["distribution"]
                    ["url"] = json!("https://example.com/node.tar.gz")
            }
        }
        fs::write(repo.path().join("turbo.lock"), value.to_string()).unwrap();
        let result = ActivationPlan::inspect(repo.path(), context());
        if change == 3 {
            assert!(matches!(result, Err(Error::InvalidLock)));
        } else {
            assert!(matches!(result, Err(Error::UnsupportedAdapter)));
        }
    }
}

#[test]
fn node_only_selection_uses_the_same_readiness_and_snapshot_contract() {
    let repo = tempfile::tempdir().unwrap();
    let mut value = fixture();
    value["tools"].as_object_mut().unwrap().remove("pnpm");
    write_sources(repo.path(), &value);
    fs::write(repo.path().join("package.json"), "{}").unwrap();
    seed(repo.path(), &desired(&value, None));
    let plan = ActivationPlan::inspect(repo.path(), context()).unwrap();
    assert_eq!(plan.snapshot().tools().len(), 1);
    assert_eq!(plan.tools().len(), 1);
}
