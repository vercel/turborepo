#![allow(clippy::unwrap_used)]

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

use serde_json::{Value, json};
use turborepo_platform::{Architecture, OperatingSystem, Platform as HostPlatform};
use turborepo_tool_install::{Activation, Reuse, UserStore};

use super::*;
use crate::{
    execution_identity::Libc,
    lock::{Ecosystems, Lock},
    rust_discovery::Toolchain,
    rust_metadata::tests::fixture as manifest,
    rust_resolution,
};

const TOOLCHAIN: &str = "[toolchain]\nchannel = \"1.99\"\nprofile = \"minimal\"\n";
const RUST: Ecosystems = Ecosystems {
    rust: true,
    python: false,
    go: false,
};

fn context() -> ExecutionContext {
    ExecutionContext::new(
        HostPlatform::new(OperatingSystem::Macos, Architecture::Arm64),
        Libc::None,
        "aarch64-apple-darwin".into(),
        BTreeSet::from(["baseline".into()]),
    )
    .unwrap()
}

fn inspect(repo: &Path, context: ExecutionContext) -> Result<ActivationPlan, Error> {
    inspect_with(repo, context, Ecosystems::default())
}

fn inspect_with(
    repo: &Path,
    context: ExecutionContext,
    ecosystems: Ecosystems,
) -> Result<ActivationPlan, Error> {
    let sources = Snapshot::capture_with(repo, ecosystems)?;
    ActivationPlan::captured(&sources, context)
}

fn inspect_clone_local(
    repo: &Path,
    context: ExecutionContext,
    recorded: &[u8],
) -> Result<ActivationPlan, Error> {
    let sources = Snapshot::capture(repo)?;
    ActivationPlan::clone_local(&sources, context, recorded)
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
    let platform = Platform::MacosArm64;
    let node = lock
        .tools()
        .contains_key("node")
        .then(|| NodePlan::from_lock(&lock, platform).unwrap());
    let mut tools = Vec::new();
    if let Some(node) = &node {
        tools.push(node.inventory_tool().clone());
        if lock.tools().contains_key("pnpm") {
            let pnpm = PnpmPlan::from_lock(&lock, platform, node, authored).unwrap();
            tools.push(pnpm.inventory_tool().clone());
        }
    }
    if lock.tools().contains_key("rust") {
        let rust = RustPlan::from_lock(&lock, platform).unwrap();
        let rustup = RustupPlan::from_lock(&lock, platform).unwrap();
        tools.extend([
            rust.inventory_tool().clone(),
            rustup.inventory_tool().clone(),
        ]);
    }
    if lock.tools().contains_key("yarn") {
        tools.push(
            YarnPlan::from_lock(
                &lock,
                Platform::MacosArm64,
                node.as_ref().unwrap(),
                authored,
            )
            .unwrap()
            .inventory_tool()
            .clone(),
        );
    }
    if lock.tools().contains_key("bun") {
        tools.push(
            BunPlan::from_lock(&lock, Platform::MacosArm64)
                .unwrap()
                .inventory_tool()
                .clone(),
        );
    }
    tools.sort_by(|a, b| a.id.cmp(&b.id));
    tools
}

/// Lock rustup and the exact 1.99.0 toolchain beside `value`'s tools, with
/// the matching root toolchain file.
fn with_rust(repo: &Path, mut value: Value) -> Value {
    fs::write(repo.join("rust-toolchain.toml"), TOOLCHAIN).unwrap();
    let toolchain = Toolchain::parse("rust-toolchain.toml", TOOLCHAIN).unwrap();
    let text = manifest("1.99.0", "2026-10-01");
    let rust = rust_resolution::resolve(&toolchain, text.as_bytes()).unwrap();
    value["tools"]["rust"] = serde_json::to_value(rust).unwrap();
    value["tools"]["rustup"] = json!({"adapter":"rustup","version":"1.29.1",
    "declarations":[{"file":"rust-toolchain.toml"}],
    "installation":{"kind":"managed","artifacts":{"macos-arm64":{"rustup-init":{
        "url":"https://static.rust-lang.org/rustup/archive/1.29.1/aarch64-apple-darwin/rustup-init",
        "sha256":"55".repeat(32),"format":"binary","executables":{"rustup":"bin/rustup"}
    }}}}});
    fs::write(repo.join("turbo.lock"), value.to_string()).unwrap();
    value
}

/// Replace pnpm with an exact Yarn classic, modern Yarn, or Bun selection in
/// the official lock shape, returning the matching `packageManager` value.
fn with_manager(id: &str, version: &str) -> (Value, String) {
    let request = format!("{id}@{version}");
    let declarations = json!([{"file":"package.json","field":"/packageManager","request":request}]);
    let installation = if id == "bun" {
        json!({"kind":"managed","artifacts":{"macos-arm64":{"distribution":{
            "url":format!("https://github.com/oven-sh/bun/releases/download/bun-v{version}/bun-darwin-aarch64.zip"),
            "sha256":"33".repeat(32),"format":"zip","rootPrefix":"bun-darwin-aarch64",
            "executables":{"bun":"bun","bunx":"bun"}
        }}}})
    } else {
        let tarball = if version.starts_with("1.") {
            format!("yarn/-/yarn-{version}.tgz")
        } else {
            format!("@yarnpkg/cli-dist/-/cli-dist-{version}.tgz")
        };
        json!({"kind":"managed","artifacts":{"any":{"package":{
            "url":format!("https://registry.npmjs.org/{tarball}"),
            "sha256":"44".repeat(32),"format":"tar-gz","rootPrefix":"package",
            "executables":{"yarn":"bin/yarn.js","yarnpkg":"bin/yarn.js"}
        }}}})
    };
    let mut value = fixture();
    let tools = value["tools"].as_object_mut().unwrap();
    tools.remove("pnpm");
    tools.insert(
        id.into(),
        json!({"adapter":id,"version":version,"declarations":declarations,"installation":installation}),
    );
    (value, request)
}

fn manager_sources(repo: &Path, value: &Value, package_manager: &str) {
    write_sources(repo, value);
    fs::write(
        repo.join("package.json"),
        json!({ "packageManager": package_manager }).to_string(),
    )
    .unwrap();
}

/// A bin export launches the staged tree program: the trap `stage` writes.
fn assert_runs_tree_trap(export: &Path) {
    let cwd = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(export)
        .current_dir(cwd.path())
        .env_clear()
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(99));
    assert!(cwd.path().join("activation-was-probed").exists());
}

// Readiness fixtures only, not executable authorization or vendor verification.
// Every export is a trap: any probe would fail and leave an observable marker.
fn stage(tool: &Tool, root: &Path) -> Result<(), turborepo_tool_install::Error> {
    for path in tool.tree_files() {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, "#!/bin/sh\ntouch activation-was-probed\nexit 99\n")?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    if tool.id == "node" && tool.executables.contains_key("npm") {
        let npm = root.join("lib/node_modules/npm");
        fs::create_dir_all(npm.join("bin"))?;
        fs::write(
            npm.join("package.json"),
            r#"{"name":"npm","version":"11.0.0","bin":{"npm":"bin/npm-cli.js","npx":"bin/npx-cli.js"}}"#,
        )?;
        for name in ["npm", "npx"] {
            let cli = format!("bin/{name}-cli.js");
            fs::rename(root.join(format!("bin/{name}")), npm.join(&cli))?;
            symlink(
                format!("../lib/node_modules/npm/{cli}"),
                root.join(format!("bin/{name}")),
            )?;
        }
    }
    fs::create_dir_all(root.join("resources"))?;
    fs::write(
        root.join("resources/template.json"),
        "adjacent runtime resource",
    )?;
    Ok(())
}

/// A plan's selection installed into a fixture tool store, as setup installs
/// it, and activated for its repository, as run activates it.
struct Installed {
    _home: tempfile::TempDir,
    store: UserStore,
    activation: Activation,
}

impl Installed {
    fn bin(&self) -> &Path {
        self.activation.bin()
    }

    fn vars(&self) -> &BTreeMap<String, OsString> {
        self.activation.vars()
    }

    /// A tree path as activation resolves it, which the tree trap proves.
    fn tree(&self, var: &str) -> PathBuf {
        PathBuf::from(&self.vars()[var])
    }
}

fn activate(repo: &Path, plan: &ActivationPlan) -> Installed {
    let home = tempfile::tempdir().unwrap();
    let store = UserStore::at(&home.path().join("store"), repo).unwrap();
    store
        .begin_setup(repo, Reuse::Sealed)
        .unwrap()
        .ensure(plan.selection(), stage, |_| Ok(()))
        .unwrap();
    let root = repo.canonicalize().unwrap();
    let activation = store.activate(&root, plan.selection().key()).unwrap();
    Installed {
        _home: home,
        store,
        activation,
    }
}

/// The Node/pnpm fixture's sources; planning needs nothing installed.
fn installed() -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    write_sources(repo.path(), &fixture());
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
    let before = state(repo.path());
    let env: BTreeMap<_, _> = std::env::vars_os().collect();
    let plan = inspect(repo.path(), context()).unwrap();
    assert_eq!(plan.tools(), desired(&fixture(), None));
    assert_eq!(plan.snapshot().tools().len(), 2);
    assert_eq!(state(repo.path()), before);
    assert_eq!(std::env::vars_os().collect::<BTreeMap<_, _>>(), env);
    let installed = activate(repo.path(), &plan);
    assert!(installed.vars().is_empty());
    let inherited = OsStr::new("/host/bin::/other bin");
    let path = installed.activation.path(Some(inherited)).unwrap();
    let paths: Vec<_> = std::env::split_paths(&path).collect();
    assert_eq!(paths[0], installed.bin());
    assert_eq!(
        &paths[1..],
        &[
            PathBuf::from("/host/bin"),
            PathBuf::new(),
            PathBuf::from("/other bin")
        ]
    );
    assert_eq!(
        installed.activation.path(None).unwrap(),
        installed.bin().as_os_str()
    );
}

#[test]
fn captured_inspection_preserves_original_root_and_source_bytes() {
    for case in 0..3 {
        let repo = installed();
        let sources = Snapshot::capture(repo.path()).unwrap();
        let before = state(repo.path());
        let plan = ActivationPlan::captured(&sources, context()).unwrap();
        assert_eq!(plan.tools(), desired(&fixture(), None));
        assert_eq!(state(repo.path()), before);
        match case {
            0 => fs::write(repo.path().join("turbo.lock"), format!("{} ", fixture())).unwrap(),
            1 => fs::write(repo.path().join(".nvmrc"), "24.0.0 ").unwrap(),
            _ => {
                let moved = tempfile::tempdir().unwrap();
                fs::rename(repo.path(), moved.path().join("original")).unwrap();
                fs::create_dir(repo.path()).unwrap();
                write_sources(repo.path(), &fixture());
            }
        }
        let after_edit = state(repo.path());
        assert!(matches!(
            ActivationPlan::captured(&sources, context()),
            Err(Error::Sources(StorageError::Conflict))
        ));
        assert_eq!(state(repo.path()), after_edit);
    }
}

#[test]
fn compatible_relocation_keeps_the_snapshot_and_the_environment() {
    let repo = installed();
    let before = inspect(repo.path(), context()).unwrap();
    let relocated = tempfile::tempdir().unwrap();
    let target = relocated.path().join("repo ' relocated");
    fs::rename(repo.path(), &target).unwrap();
    let after = inspect(&target, context()).unwrap();
    assert_eq!(before.selection().key(), after.selection().key());
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
fn missing_lock_never_uses_ancestors_and_planning_creates_nothing() {
    let parent = installed();
    let repo = parent.path().join("nested");
    fs::create_dir(&repo).unwrap();
    let before = state(&repo);
    assert!(matches!(inspect(&repo, context()), Err(Error::MissingLock)));
    assert_eq!(state(&repo), before);
    write_sources(&repo, &fixture());
    let before = state(&repo);
    inspect(&repo, context()).unwrap();
    assert_eq!(state(&repo), before);
    assert!(!repo.join(".turbo").exists());
}

#[test]
fn any_other_tool_set_names_another_environment() {
    let repo = installed();
    let plan = inspect(repo.path(), context()).unwrap();
    for change in 0..4 {
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
                vars: Default::default(),
            }),
        }
        let other = turborepo_tool_install::Selection::new(tools).unwrap();
        assert_ne!(other.key(), plan.selection().key(), "{change}");
    }
}

#[test]
fn native_provenance_and_capture_changes_are_rejected() {
    let repo = installed();
    let captured = Snapshot::capture(repo.path()).unwrap();
    fs::write(repo.path().join(".nvmrc"), "24.x").unwrap();
    let before = state(repo.path());
    assert!(matches!(
        inspect(repo.path(), context()),
        Err(Error::DeclarationDrift)
    ));
    assert!(matches!(
        ActivationPlan::captured(&captured, context()),
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
        let plan = inspect(repo.path(), context()).unwrap();
        assert_ne!(plan.tools(), desired(&value, None));
        let authored = crate::package_manager::CorepackIntegrity {
            algorithm: "sha512",
            digest: "aa".repeat(64),
        };
        assert_eq!(plan.tools(), desired(&value, Some(&authored)));
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
            matches!(inspect(repo.path(), context()), Err(Error::InvalidLock)),
            "{manifest}"
        );
        assert_eq!(state(repo.path()), before);
        assert!(!repo.path().join(".turbo").exists());
    }
}

fn linux(libc: Libc) -> ExecutionContext {
    let host = HostPlatform::new(OperatingSystem::Linux, Architecture::X64);
    ExecutionContext::new(
        host,
        libc,
        "target".into(),
        BTreeSet::from(["baseline".into()]),
    )
    .unwrap()
}

fn musl(abi: &str) -> ExecutionContext {
    linux(Libc::Musl { abi: abi.into() })
}

#[test]
fn unsupported_adapters_platforms_and_invalid_lock_metadata_are_distinct() {
    let repo = installed();
    let windows_arm64 = ExecutionContext::new(
        HostPlatform::new(OperatingSystem::Windows, Architecture::Arm64),
        Libc::None,
        "target".into(),
        BTreeSet::from(["baseline".into()]),
    )
    .unwrap();
    assert!(matches!(
        inspect(repo.path(), windows_arm64),
        Err(Error::UnsupportedPlatform)
    ));
    // musl hosts never activate managed artifacts or clone-local generations.
    assert!(matches!(
        inspect(repo.path(), musl("1.2.5")),
        Err(Error::MuslHost)
    ));
    let selection = Lock::parse(fixture().to_string().as_bytes()).unwrap();
    assert!(matches!(
        inspect_clone_local(
            repo.path(),
            musl("1.2.5"),
            &selection.canonical_bytes().unwrap()
        ),
        Err(Error::MuslHost)
    ));
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
        let result = inspect(repo.path(), context());
        if change == 3 {
            assert!(matches!(result, Err(Error::InvalidLock)));
        } else if change < 2 {
            assert!(matches!(
                result,
                Err(Error::Sources(StorageError::Lock(
                    crate::lock::Error::RemovedInstallation
                )))
            ));
        } else {
            assert!(matches!(result, Err(Error::UnsupportedAdapter)));
        }
    }
}

#[test]
fn locked_node_semantics_are_checked_before_inventory() {
    for (file, contents, version) in [
        (".nvmrc", "26.0.0", "24.0.0"),
        (".node-version", "26.x", "24.0.0"),
        ("package.json", r#"{"engines":{"node":">=26"}}"#, "24.0.0"),
        (
            "package.json",
            r#"{"devEngines":{"runtime":{"name":"node","version":">=26"}}}"#,
            "24.0.0",
        ),
        (".nvmrc", "lts/*", "24.0.0-rc.1"),
    ] {
        let repo = tempfile::tempdir().unwrap();
        let mut value = fixture();
        value["tools"].as_object_mut().unwrap().remove("pnpm");
        write_sources(repo.path(), &value);
        fs::remove_file(repo.path().join(".nvmrc")).unwrap();
        fs::write(repo.path().join("package.json"), "{}").unwrap();
        fs::write(repo.path().join(file), contents).unwrap();
        let captured = Snapshot::capture(repo.path()).unwrap();
        value["tools"]["node"]["declarations"] =
            serde_json::to_value(&captured.declarations()["node"]).unwrap();
        value["tools"]["node"]["version"] = json!(version);
        let artifact =
            &mut value["tools"]["node"]["installation"]["artifacts"]["macos-arm64"]["distribution"];
        artifact["url"] = json!(format!(
            "https://nodejs.org/dist/v{version}/node-v{version}-darwin-arm64.tar.gz"
        ));
        artifact["rootPrefix"] = json!(format!("node-v{version}-darwin-arm64"));
        fs::write(repo.path().join("turbo.lock"), value.to_string()).unwrap();
        let before = state(repo.path());
        assert!(
            matches!(
                inspect(repo.path(), context()),
                Err(Error::DeclarationDrift)
            ),
            "{file}: {contents}"
        );
        assert_eq!(state(repo.path()), before);
    }
}

#[test]
fn node_only_selection_uses_the_same_readiness_and_snapshot_contract() {
    let repo = tempfile::tempdir().unwrap();
    let mut value = fixture();
    value["tools"].as_object_mut().unwrap().remove("pnpm");
    write_sources(repo.path(), &value);
    fs::write(repo.path().join("package.json"), "{}").unwrap();
    let plan = inspect(repo.path(), context()).unwrap();
    assert_eq!(plan.snapshot().tools().len(), 1);
    assert_eq!(plan.tools(), desired(&value, None));
    value["tools"]["node"]["options"]["bundled-npm"] = json!(["12.0.0"]);
    fs::write(repo.path().join("turbo.lock"), value.to_string()).unwrap();
    let before = state(repo.path());
    // Setup verifies the bundled npm against the tree it installs; the lock
    // digest in the repository's record binds the option for run.
    let changed = inspect(repo.path(), context()).unwrap();
    assert_eq!(changed.selection().key(), plan.selection().key());
    assert_eq!(state(repo.path()), before);
}

const PYTHON: Ecosystems = Ecosystems {
    python: true,
    rust: false,
    go: false,
};
const PYPROJECT: &str =
    "[project]\nname = \"app\"\nversion = \"0.1.0\"\n[tool.uv]\nrequired-version = \">=0.13\"\n";

/// Adds root uv (and optionally CPython) selections for the test context.
fn with_python(mut value: Value, python: bool) -> Value {
    let uv =
        crate::uv::UvArtifact::for_platform(&semver::Version::new(0, 13, 0), Platform::MacosArm64)
            .unwrap()
            .lock_artifact("33".repeat(32));
    value["tools"]["uv"] = json!({"adapter":"uv","version":"0.13.0","declarations":[],
        "installation":{"kind":"managed","artifacts":{"macos-arm64":{"distribution":uv}}}});
    if python {
        let artifact = crate::python_resolution::official_artifact(
            &semver::Version::new(3, 12, 12),
            Platform::MacosArm64,
            "20260211/cpython-3.12.12%2B20260211-aarch64-apple-darwin-install_only_stripped.tar.gz",
            "44".repeat(32),
        )
        .unwrap();
        value["tools"]["python"] = json!({"adapter":"python","version":"3.12.12",
            "declarations":[],
            "installation":{"kind":"managed","artifacts":{"macos-arm64":{"distribution":artifact}}}});
    }
    value
}

/// Writes root uv/Python declarations and binds the lock to them exactly as
/// setup captures them.
fn python_installed(mut value: Value) -> (tempfile::TempDir, Value) {
    let repo = tempfile::tempdir().unwrap();
    if value["tools"].get("node").is_some() {
        write_sources(repo.path(), &fixture());
    }
    fs::write(repo.path().join("pyproject.toml"), PYPROJECT).unwrap();
    if value["tools"].get("python").is_some() {
        fs::write(repo.path().join(".python-version"), "3.12\n").unwrap();
    }
    let captured = Snapshot::capture_with(repo.path(), PYTHON).unwrap();
    for id in ["uv", "python"] {
        if value["tools"].get(id).is_some() {
            value["tools"][id]["declarations"] =
                serde_json::to_value(&captured.declarations()[id]).unwrap();
        }
    }
    fs::write(repo.path().join("turbo.lock"), value.to_string()).unwrap();
    (repo, value)
}

#[test]
fn uv_and_python_export_bins_and_pin_uv_to_the_locked_interpreter() {
    let (repo, _) = python_installed(with_python(fixture(), true));
    let before = state(repo.path());
    let plan = inspect_with(repo.path(), context(), PYTHON).unwrap();
    let ids: Vec<_> = plan.tools().iter().map(|tool| tool.id.as_str()).collect();
    assert_eq!(ids, ["node", "pnpm", "python", "uv"]);
    assert_eq!(plan.snapshot().tools().len(), 4);
    assert_eq!(state(repo.path()), before);
    let installed = activate(repo.path(), &plan);
    for name in ["node", "pnpm", "python", "python3", "uv", "uvx"] {
        assert!(installed.bin().join(name).is_file(), "{name}");
    }
    let tree = installed.tree("UV_PYTHON_INSTALL_DIR");
    assert!(tree.join("cpython-3.12.12-macos-aarch64-none").is_dir());
    assert!(
        fs::canonicalize(installed.bin().join("python3"))
            .unwrap()
            .starts_with(fs::canonicalize(&tree).unwrap())
    );
    assert_eq!(
        installed.vars(),
        &BTreeMap::from([
            (
                "UV_PYTHON".into(),
                "cpython-3.12.12-macos-aarch64-none".into()
            ),
            ("UV_PYTHON_DOWNLOADS".into(), "never".into()),
            ("UV_PYTHON_INSTALL_DIR".into(), tree.into_os_string()),
            ("UV_PYTHON_PREFERENCE".into(), "only-managed".into()),
        ])
    );
}

#[test]
fn python_only_uv_mirrors_its_launchers_without_selecting_an_interpreter() {
    let (repo, _) = python_installed(with_python(json!({"schemaVersion":0,"tools":{}}), false));
    let plan = inspect_with(repo.path(), context(), PYTHON).unwrap();
    assert_eq!(plan.tools().len(), 1);
    let installed = activate(repo.path(), &plan);
    assert_eq!(installed.vars().len(), 2);
    assert_eq!(
        installed.vars()["UV_PYTHON_DOWNLOADS"],
        OsString::from("never")
    );
    // Never the user's own uv-managed interpreters: a path that never exists.
    let none = installed.tree("UV_PYTHON_INSTALL_DIR");
    assert!(none.starts_with(installed.store.root().join("envs")));
    assert!(!none.exists());
}

#[test]
fn uv_and_python_fail_closed_when_disabled_or_drifted() {
    for change in 0..3 {
        let (repo, _) = python_installed(with_python(fixture(), true));
        match change {
            // experimentalPythonWorkspaces off: selections are not activated.
            0 => {}
            1 => fs::write(repo.path().join(".python-version"), "3.13\n").unwrap(),
            _ => fs::write(
                repo.path().join("pyproject.toml"),
                PYPROJECT.replace(">=0.13", "<0.13"),
            )
            .unwrap(),
        }
        let before = state(repo.path());
        let result = if change == 0 {
            inspect(repo.path(), context())
        } else {
            inspect_with(repo.path(), context(), PYTHON)
        };
        assert!(
            matches!(
                (change, result),
                (0, Err(Error::UnsupportedAdapter)) | (1 | 2, Err(Error::DeclarationDrift))
            ),
            "{change}"
        );
        assert_eq!(state(repo.path()), before);
    }
}

#[test]
fn yarn_and_bun_contribute_exports_through_the_node_and_pnpm_plan_shape() {
    for (id, version, exports) in [
        ("yarn", "1.22.22", ["yarn", "yarnpkg"]),
        ("yarn", "4.5.0", ["yarn", "yarnpkg"]),
        ("bun", "1.2.23", ["bun", "bunx"]),
    ] {
        let (value, request) = with_manager(id, version);
        let repo = tempfile::tempdir().unwrap();
        manager_sources(repo.path(), &value, &request);
        let tools = desired(&value, None);
        let before = state(repo.path());
        let plan = inspect(repo.path(), context()).unwrap();
        assert_eq!(plan.tools(), tools, "{request}");
        assert_eq!(plan.snapshot().tools().len(), 2);
        assert_eq!(state(repo.path()), before);
        // Same representation as Node/pnpm: one managed bin, no runtime values.
        let installed = activate(repo.path(), &plan);
        assert!(installed.vars().is_empty());
        for name in ["node", "npm", "npx"].into_iter().chain(exports) {
            assert!(installed.bin().join(name).exists(), "{request}: {name}");
        }
        assert!(!installed.bin().join("pnpm").exists());
    }
}

#[test]
fn rust_and_python_cohort_preserves_both_private_runtime_environments() {
    let (repo, value) = python_installed(with_python(fixture(), true));
    let python_env = activate(
        repo.path(),
        &inspect_with(repo.path(), context(), PYTHON).unwrap(),
    )
    .vars()
    .clone();
    let value = with_rust(repo.path(), value);
    let lock = Lock::parse(value.to_string().as_bytes()).unwrap();
    let mut tools = desired(&value, None);
    tools.push(
        UvPlan::from_lock(&lock, Platform::MacosArm64)
            .unwrap()
            .inventory_tool()
            .clone(),
    );
    tools.push(
        PythonPlan::from_lock(&lock, Platform::MacosArm64)
            .unwrap()
            .inventory_tool()
            .clone(),
    );
    tools.sort_by(|a, b| a.id.cmp(&b.id));
    let before = state(repo.path());
    let plan = inspect_with(
        repo.path(),
        context(),
        Ecosystems {
            rust: true,
            python: true,
            go: false,
        },
    )
    .unwrap();
    assert_eq!(plan.tools(), tools);
    assert_eq!(plan.snapshot().tools().len(), 6);
    assert_eq!(state(repo.path()), before);
    let installed = activate(repo.path(), &plan);
    for (name, value) in python_env {
        if name != "UV_PYTHON_INSTALL_DIR" {
            assert_eq!(installed.vars().get(&name), Some(&value));
        }
    }
    let python = installed.tree("UV_PYTHON_INSTALL_DIR");
    assert!(python.join("cpython-3.12.12-macos-aarch64-none").is_dir());
    assert_eq!(installed.vars().len(), 7);
    assert_eq!(
        installed.vars()["RUSTUP_TOOLCHAIN"],
        OsString::from("1.99.0-aarch64-apple-darwin")
    );
    let root = repo.path().canonicalize().unwrap();
    assert_eq!(
        installed.vars()["CARGO_HOME"],
        installed.store.cargo_home(&root).into_os_string()
    );
    assert!(
        installed
            .tree("RUSTUP_HOME")
            .starts_with(installed.store.root().join("trees"))
    );
    assert!(!repo.path().join(".turbo/cargo-home").exists());
}

#[test]
fn rust_selection_adds_toolchain_exports_and_private_runtime_env() {
    let repo = tempfile::tempdir().unwrap();
    write_sources(repo.path(), &fixture());
    let value = with_rust(repo.path(), fixture());
    let before = state(repo.path());
    // Without the Cargo flag the Rust selections are drift, never ignored.
    assert!(matches!(
        inspect(repo.path(), context()),
        Err(Error::DeclarationDrift)
    ));
    let plan = inspect_with(repo.path(), context(), RUST).unwrap();
    assert_eq!(plan.tools(), desired(&value, None));
    let ids: Vec<_> = plan.snapshot().tools().keys().collect();
    assert_eq!(ids, ["node", "pnpm", "rust", "rustup"]);
    assert_eq!(state(repo.path()), before);
    let installed = activate(repo.path(), &plan);
    let root = repo.path().canonicalize().unwrap();
    let cargo_home = installed.store.cargo_home(&root);
    let home = installed.tree("RUSTUP_HOME");
    assert!(home.is_dir() && home.starts_with(installed.store.root().join("trees")));
    let toolchain = "1.99.0-aarch64-apple-darwin";
    assert_eq!(
        installed.vars(),
        &BTreeMap::from([
            ("CARGO_HOME".into(), cargo_home.clone().into_os_string()),
            ("RUSTUP_HOME".into(), home.into_os_string()),
            ("RUSTUP_TOOLCHAIN".into(), toolchain.into()),
        ])
    );
    for name in ["cargo", "rustc", "rustdoc"] {
        assert_runs_tree_trap(&installed.bin().join(name));
    }
    // Cargo creates its home on first use; activation never does.
    assert!(!root.join(".turbo/cargo-home").exists());
    assert!(!cargo_home.exists());
}

#[test]
fn rust_only_plans_bind_the_toolchain_file_and_cargo_home() {
    let repo = tempfile::tempdir().unwrap();
    with_rust(repo.path(), json!({"schemaVersion":0,"tools":{}}));
    let plan = inspect_with(repo.path(), context(), RUST).unwrap();
    let ids: Vec<_> = plan.tools().iter().map(|tool| tool.id.as_str()).collect();
    assert_eq!(ids, ["rust", "rustup"]);
    let installed = activate(repo.path(), &plan);
    let root = repo.path().canonicalize().unwrap();
    let home = installed.store.cargo_home(&root);
    let activate = || installed.store.activate(&root, plan.selection().key());

    // An existing private home is used; a redirected or non-directory one is not.
    fs::create_dir_all(&home).unwrap();
    assert!(activate().is_ok());
    fs::remove_dir(&home).unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), &home).unwrap();
    assert!(matches!(
        activate(),
        Err(turborepo_tool_install::Error::UnsafePath)
    ));
    fs::remove_file(&home).unwrap();
    fs::write(&home, "").unwrap();
    assert!(matches!(
        activate(),
        Err(turborepo_tool_install::Error::UnsafePath)
    ));
    fs::remove_file(&home).unwrap();

    // Toolchain edits are drift.
    let file = repo.path().join("rust-toolchain.toml");
    for edit in [
        TOOLCHAIN.replace("1.99", "1.98"),
        TOOLCHAIN.replace("minimal\"", "minimal\"\ncomponents = [\"clippy\"]"),
    ] {
        fs::write(&file, edit).unwrap();
        let before = state(repo.path());
        assert!(matches!(
            inspect_with(repo.path(), context(), RUST),
            Err(Error::DeclarationDrift)
        ));
        assert_eq!(state(repo.path()), before);
    }
}

#[test]
fn switched_or_missing_manager_installations_never_activate() {
    let (yarn, yarn_request) = with_manager("yarn", "4.5.0");
    let (bun, bun_request) = with_manager("bun", "1.2.23");
    let installed = desired(&yarn, None);
    let yarn_env = turborepo_tool_install::Selection::new(installed.clone()).unwrap();
    let node_env = turborepo_tool_install::Selection::new(installed[..1].to_vec()).unwrap();
    // The declaration moved to Bun after setup; the lock still selects Yarn.
    let repo = tempfile::tempdir().unwrap();
    manager_sources(repo.path(), &yarn, &bun_request);
    let before = state(repo.path());
    assert!(matches!(
        inspect(repo.path(), context()),
        Err(Error::DeclarationDrift)
    ));
    assert_eq!(state(repo.path()), before);
    // Lock and declaration select Bun: neither the Yarn environment setup
    // installed nor a Node-only one.
    manager_sources(repo.path(), &bun, &bun_request);
    let plan = inspect(repo.path(), context()).unwrap();
    for other in [&yarn_env, &node_env] {
        assert_ne!(plan.selection().key(), other.key());
    }
    manager_sources(repo.path(), &yarn, &yarn_request);
    assert_eq!(
        inspect(repo.path(), context()).unwrap().selection().key(),
        yarn_env.key()
    );
}

#[test]
fn modern_yarn_bundle_pin_is_part_of_the_installed_binding() {
    let (mut value, _) = with_manager("yarn", "4.5.0");
    let repo = tempfile::tempdir().unwrap();
    let digest = "aa".repeat(64);
    manager_sources(repo.path(), &value, &format!("yarn@4.5.0+sha512.{digest}"));
    let captured = Snapshot::capture(repo.path()).unwrap();
    value["tools"]["yarn"]["declarations"] =
        serde_json::to_value(&captured.declarations()["yarn"]).unwrap();
    fs::write(repo.path().join("turbo.lock"), value.to_string()).unwrap();
    let plan = inspect(repo.path(), context()).unwrap();
    assert_ne!(plan.tools(), desired(&value, None));
    let authored = crate::package_manager::CorepackIntegrity {
        algorithm: "sha512",
        digest,
    };
    assert_eq!(plan.tools(), desired(&value, Some(&authored)));
}

/// A lock selecting `id` alone, with no Node declaration in the sources.
fn without_node(repo: &Path, id: &str, version: &str) -> Value {
    let (mut value, request) = with_manager(id, version);
    value["tools"].as_object_mut().unwrap().remove("node");
    manager_sources(repo, &value, &request);
    fs::remove_file(repo.join(".nvmrc")).unwrap();
    value
}

#[test]
fn bun_only_selection_activates_without_managed_node() {
    let repo = tempfile::tempdir().unwrap();
    let value = without_node(repo.path(), "bun", "1.2.23");
    let tools = desired(&value, None);
    let before = state(repo.path());
    let plan = inspect(repo.path(), context()).unwrap();
    assert_eq!(plan.tools(), tools);
    assert_eq!(plan.snapshot().tools().len(), 1);
    assert_eq!(state(repo.path()), before);
    let installed = activate(repo.path(), &plan);
    assert!(installed.vars().is_empty());
    for name in ["bun", "bunx"] {
        assert!(installed.bin().join(name).exists());
    }
    for name in ["node", "npm", "npx"] {
        assert!(!installed.bin().join(name).exists());
    }
    // A later Node declaration is drift until setup locks it.
    fs::write(repo.path().join(".nvmrc"), "24.0.0\n").unwrap();
    assert!(matches!(
        inspect(repo.path(), context()),
        Err(Error::DeclarationDrift)
    ));
}

#[test]
fn node_based_managers_never_activate_without_managed_node() {
    for (id, version) in [("pnpm", "10.0.0"), ("yarn", "1.22.22"), ("yarn", "4.5.0")] {
        let repo = tempfile::tempdir().unwrap();
        if id == "pnpm" {
            let mut value = fixture();
            value["tools"].as_object_mut().unwrap().remove("node");
            write_sources(repo.path(), &value);
            fs::remove_file(repo.path().join(".nvmrc")).unwrap();
        } else {
            without_node(repo.path(), id, version);
        }
        let before = state(repo.path());
        assert!(
            matches!(inspect(repo.path(), context()), Err(Error::InvalidLock)),
            "{id} {version}"
        );
        assert_eq!(state(repo.path()), before);
    }
}

const GO: Ecosystems = Ecosystems {
    python: false,
    rust: false,
    go: true,
};

/// A `go 1.22` repository locked to 1.22.3, optionally beside the
/// Node/pnpm fixture.
fn go_installed(js: bool) -> (tempfile::TempDir, Value) {
    let repo = tempfile::tempdir().unwrap();
    let mut value = if js {
        fixture()
    } else {
        json!({"schemaVersion":0,"tools":{}})
    };
    value["tools"]["go"] = json!({"adapter":"go","version":"1.22.3",
    "declarations":[{"file":"go.work","field":"go","request":"1.22"}],
    "installation":{"kind":"managed","artifacts":{"macos-arm64":{"distribution":{
        "url":"https://dl.google.com/go/go1.22.3.darwin-arm64.tar.gz",
        "sha256":"33".repeat(32),"format":"tar-gz","rootPrefix":"go",
        "executables":{"go":"bin/go","gofmt":"bin/gofmt"}
    }}}}});
    write_sources(repo.path(), &value);
    if !js {
        fs::remove_file(repo.path().join(".nvmrc")).unwrap();
        fs::remove_file(repo.path().join("package.json")).unwrap();
    }
    fs::write(repo.path().join("go.work"), "go 1.22\n").unwrap();
    (repo, value)
}

fn go_desired(value: &Value) -> Vec<Tool> {
    let lock = Lock::parse(value.to_string().as_bytes()).unwrap();
    let mut tools = if lock.tools().contains_key("node") {
        desired(value, None)
    } else {
        Vec::new()
    };
    let go = GoPlan::from_lock(&lock, Platform::MacosArm64).unwrap();
    tools.push(go.inventory_tool().clone());
    tools.sort_by(|a, b| a.id.cmp(&b.id));
    tools
}

#[test]
fn go_selection_contributes_its_bin_and_locked_runtime_env() {
    for js in [false, true] {
        let (repo, value) = go_installed(js);
        let before = state(repo.path());
        let plan = inspect_with(repo.path(), context(), GO).unwrap();
        assert_eq!(plan.tools(), go_desired(&value));
        assert_eq!(plan.snapshot().tools().len(), if js { 3 } else { 1 });
        assert_eq!(state(repo.path()), before);
        let installed = activate(repo.path(), &plan);
        let goroot = installed.tree("GOROOT");
        assert!(goroot.join("resources/template.json").is_file());
        assert_eq!(
            installed.vars(),
            &BTreeMap::from([
                ("GOROOT".to_owned(), goroot.into_os_string()),
                ("GOTOOLCHAIN".to_owned(), OsString::from("local")),
            ])
        );
        for name in ["go", "gofmt"] {
            assert_runs_tree_trap(&installed.bin().join(name));
        }
    }
    // A compatible relocation keeps the selected identity.
    let (repo, _) = go_installed(false);
    let before = inspect_with(repo.path(), context(), GO).unwrap();
    let relocated = tempfile::tempdir().unwrap();
    let target = relocated.path().join("moved");
    fs::rename(repo.path(), &target).unwrap();
    let after = inspect_with(&target, context(), GO).unwrap();
    assert_eq!(before.selection().key(), after.selection().key());
    assert_eq!(
        before.snapshot().fingerprint(),
        after.snapshot().fingerprint()
    );
}

#[test]
fn go_drift_and_a_disabled_flag_fail_closed() {
    // Matching provenance cannot admit 1.22.3 under a newer minimum or a
    // different exact toolchain.
    for go_work in ["go 1.23\n", "go 1.22\ntoolchain go1.22.4\n"] {
        let (repo, mut value) = go_installed(false);
        fs::write(repo.path().join("go.work"), go_work).unwrap();
        let captured = Snapshot::capture_with(repo.path(), GO).unwrap();
        value["tools"]["go"]["declarations"] =
            serde_json::to_value(&captured.declarations()["go"]).unwrap();
        fs::write(repo.path().join("turbo.lock"), value.to_string()).unwrap();
        let before = state(repo.path());
        assert!(
            matches!(
                inspect_with(repo.path(), context(), GO),
                Err(Error::DeclarationDrift)
            ),
            "{go_work}"
        );
        assert_eq!(state(repo.path()), before);
    }
    // Without experimentalGoWorkspaces go.work is unread, so the locked Go
    // selection is drift, as in setup.
    let (repo, _) = go_installed(true);
    let before = state(repo.path());
    assert!(matches!(
        inspect(repo.path(), context()),
        Err(Error::DeclarationDrift)
    ));
    assert_eq!(state(repo.path()), before);
}

/// `fixture()`'s cohort as `turbo setup --no-lock` records it: no turbo.lock,
/// and the canonical selection as the user record keeps it.
fn clone_local() -> (tempfile::TempDir, Vec<u8>) {
    let repo = tempfile::tempdir().unwrap();
    write_sources(repo.path(), &fixture());
    fs::remove_file(repo.path().join("turbo.lock")).unwrap();
    let selection = Lock::parse(fixture().to_string().as_bytes()).unwrap();
    (repo, selection.canonical_bytes().unwrap())
}

#[test]
fn clone_local_selection_plans_like_its_lock_under_a_distinct_identity() {
    let (repo, recorded) = clone_local();
    let before = state(repo.path());
    let plan = inspect_clone_local(repo.path(), context(), &recorded).unwrap();
    assert_eq!(plan.tools(), desired(&fixture(), None));
    let identity: Value = serde_json::from_slice(plan.snapshot().canonical_identity()).unwrap();
    assert_eq!(identity["resolution"], "clone-local");
    assert!(matches!(
        inspect(repo.path(), context()),
        Err(Error::MissingLock)
    ));
    assert_eq!(state(repo.path()), before);
    // Committed, the same selection is the lock's: same tools and shared
    // environment, other identity, and the clone-local record no longer
    // applies.
    let lock = Lock::parse(fixture().to_string().as_bytes()).unwrap();
    fs::write(
        repo.path().join("turbo.lock"),
        lock.canonical_bytes().unwrap(),
    )
    .unwrap();
    let locked = inspect(repo.path(), context()).unwrap();
    assert_eq!(locked.tools(), plan.tools());
    assert_eq!(locked.selection().key(), plan.selection().key());
    assert_ne!(
        locked.snapshot().fingerprint(),
        plan.snapshot().fingerprint()
    );
    assert!(matches!(
        inspect_clone_local(repo.path(), context(), &recorded),
        Err(Error::Sources(StorageError::Conflict))
    ));
}

#[test]
fn clone_local_drift_and_invalid_selections_fail_closed() {
    let (repo, recorded) = clone_local();
    fs::write(repo.path().join(".nvmrc"), "24.1.0\n").unwrap();
    assert!(matches!(
        inspect_clone_local(repo.path(), context(), &recorded),
        Err(Error::DeclarationDrift)
    ));
    let (repo, _) = clone_local();
    assert!(matches!(
        inspect_clone_local(repo.path(), context(), br#"{"schemaVersion":0,"tools":{}}"#),
        Err(Error::InvalidSelection)
    ));
    assert!(!repo.path().join(".turbo").exists());
}

mod additional;
