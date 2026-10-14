//! Declared additional tools through managed run, exec, `--check`, and the
//! installation the shell hook activates. Local fixtures only.
use std::ffi::OsString;

use turbopath::AbsoluteSystemPathBuf;

use super::*;
use crate::commands::{exec, setup::trust::TrustStore};

const LOCAL: &[&str] = &["--tools-only", "--no-frozen"];
// Finds its resource beside the real executable rather than the PATH link,
// as tools such as protoc locate their bundled files.
const TOOL: &str = "#!/bin/sh\ndir=$(dirname \"$0\")/$(dirname \"$(readlink \"$0\")\")\ncat \
                    \"$dir/../share/data.txt\"\necho \" $*\"\n";

fn artifact() -> Vec<u8> {
    archive(
        "fixture-1.2.3",
        &[
            ("bin/fixture", TOOL),
            ("share/data.txt", "fixture resource"),
        ],
    )
}

fn declare(f: &Fixture, executable: &str) {
    declare_with(f, executable, &artifact());
}

fn declare_with(f: &Fixture, executable: &str, artifact: &[u8]) {
    let path = f.owned.root().join("turbo.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let platform = serde_json::to_value(provision::platform().unwrap()).unwrap();
    let tools = if executable.is_empty() {
        json!({})
    } else {
        json!({"fixture": {"version": "1.2.3", "artifacts": {platform.as_str().unwrap(): {
            "url": "https://downloads.example.test/fixture-1.2.3.tar.gz",
            "sha256": format!("{:x}", Sha256::digest(artifact)),
            "format": "tar-gz", "rootPrefix": "fixture-1.2.3",
            "executables": {executable: "bin/fixture"}}}}})
    };
    config["setup"] = json!({ "additionalTools": tools });
    fs::write(path, config.to_string()).unwrap();
}

/// Node, pnpm, and the tool installed by local setup. The task's Node runs
/// the declared tool from the task PATH.
fn prepared() -> (Fixture, LoopbackServer) {
    prepared_with("#!/bin/sh\nfixture task > managed-result\n", artifact())
}

fn prepared_with(node: &str, tool: Vec<u8>) -> (Fixture, LoopbackServer) {
    let mut f = runnable();
    let distribution = f.lock["tools"]["node"]["installation"]["artifacts"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap();
    let prefix = distribution["distribution"]["rootPrefix"]
        .as_str()
        .unwrap()
        .to_owned();
    f.node = archive(&prefix, &[("bin/node", node)]);
    distribution["distribution"]["sha256"] = json!(format!("{:x}", Sha256::digest(&f.node)));
    f.save();
    declare_with(&f, "fixture", &tool);
    let integrity = super::super::first_lock::base64(&sha2::Sha512::digest(&f.pnpm));
    let metadata = json!({"name":"pnpm","version":"10.0.0","dist":{
        "tarball":"https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz",
        "integrity":format!("sha512-{integrity}")}});
    let routes = [
        (f.node_path.clone(), f.node.clone()),
        ("/pnpm/10.0.0".into(), metadata.to_string().into_bytes()),
        ("/pnpm/-/pnpm-10.0.0.tgz".into(), f.pnpm.clone()),
        ("/fixture-1.2.3.tar.gz".into(), tool),
    ];
    let u = LoopbackServer::new(routes, |_| {}).unwrap();
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    assert_eq!(u.hits(), 4);
    (f, u)
}

#[test]
fn run_tasks_resolve_declared_tools_and_resources_in_strict_and_loose_modes() {
    for mode in ["strict", "loose"] {
        let (f, u) = prepared();
        let tools = select(&f).unwrap().unwrap();
        assert_eq!(execute(&f, mode, tools), "fixture resource task\n");
        assert_eq!(u.hits(), 4);
    }
}

#[test]
fn exec_check_and_hook_expose_the_tool_and_stale_states_never_fall_back() {
    let (f, u) = prepared();
    let root = f.owned.root();
    let cwd = AbsoluteSystemPathBuf::try_from(root.join("apps/web")).unwrap();
    let output = root.join("apps/web/exec-result");
    let launch = |mut command: std::process::Command| {
        command.stdout(fs::File::create(&output)?);
        shared_child::SharedChild::spawn(&mut command).map(Arc::new)
    };
    let command = ["fixture", "a b"].map(OsString::from);
    let tools = select(&f).unwrap().unwrap();
    assert_eq!(
        exec::spawn(tools.as_ref(), &command, &cwd, launch).unwrap(),
        0
    );
    assert_eq!(
        fs::read_to_string(&output).unwrap(),
        "fixture resource a b\n"
    );

    // `--check` accepts the locked tool without writing anything.
    assert_eq!(f.check(super::super::checks::CHECK).unwrap(), 0);
    // The hook activates the bin directory setup recorded for this root.
    let trust = TrustStore::new(f.owned.home(), &root.canonicalize().unwrap()).unwrap();
    let recorded = trust.recorded().unwrap().unwrap();
    assert!(!recorded.stale && recorded.bin.join("fixture").is_file());

    // A deleted export is a repair, never a download or a host fallback.
    fs::remove_file(recorded.bin.join("fixture")).unwrap();
    let error = select(&f).err().unwrap();
    assert!(error.contains("run turbo setup to repair"), "{error}");
    assert!(matches!(
        exec::spawn(tools.as_ref(), &command, &cwd, launch),
        Err(exec::Error::Stale(_))
    ));
    assert_eq!(u.hits(), 4);
    // Frozen setup repairs it from the unchanged lock.
    assert_eq!(f.run(&u, &["--frozen", "--tools-only"]).unwrap(), 0);
    assert!(select(&f).unwrap().is_some());

    // Removing the declaration is drift until local setup reconciles it.
    declare(&f, "");
    let hits = u.hits();
    let error = select(&f).err().unwrap();
    assert!(
        error.contains("tool declarations no longer match"),
        "{error}"
    );
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    let path = select(&f).unwrap().unwrap().revalidate().unwrap()["PATH"].clone();
    let bin = std::env::split_paths(&path).next().unwrap();
    assert!(bin.join("node").exists() && !bin.join("fixture").exists());
    assert_eq!(u.hits(), hits);
}

#[test]
fn executables_owned_by_built_in_tools_are_rejected_before_any_download() {
    let (f, u) = prepared();
    let lock = fs::read(f.owned.root().join("turbo.lock")).unwrap();
    declare(&f, "node");
    let error = format!("{:?}", f.run(&u, LOCAL).unwrap_err());
    assert!(error.contains("built-in setup adapter"), "{error}");
    let error = select(&f).err().unwrap();
    assert!(error.contains("tool declarations are invalid"), "{error}");
    assert_eq!(fs::read(f.owned.root().join("turbo.lock")).unwrap(), lock);
    assert_eq!(u.hits(), 4);
}

#[test]
fn task_hashes_follow_the_tool_artifact_and_never_the_clone_location() {
    const HASH: &str = "#!/bin/sh\nprintf '%s' \"$TURBO_HASH\" > managed-result\n";
    let mut runs = vec![];
    // The first two are separate clones with identical declarations.
    for resource in ["fixture resource", "fixture resource", "other bytes"] {
        let files = [("bin/fixture", TOOL), ("share/data.txt", resource)];
        let (f, _u) = prepared_with(HASH, archive("fixture-1.2.3", &files));
        let tools = select(&f).unwrap().unwrap();
        let identity = tools.cache_identity().to_owned();
        runs.push((identity, execute(&f, "strict", tools)));
    }
    assert!(!runs[0].1.is_empty());
    assert_eq!(runs[0], runs[1]);
    assert_ne!(runs[0].0, runs[2].0);
    assert_ne!(runs[0].1, runs[2].1);
}
