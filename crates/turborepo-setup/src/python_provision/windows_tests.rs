#![allow(clippy::unwrap_used)]
//! Native Windows x64 CPython: the verified build at uv's key directory, the
//! batch launcher behind the Store's forwarder, and bytecode compilation.

use std::{collections::BTreeMap, ffi::OsStr, io::Write, path::PathBuf};

use semver::Version;
use sha2::{Digest, Sha256};

use super::*;
use crate::{
    lock::{Declaration, Document},
    python_resolution::{RELEASES, official_artifact},
    source_policy::test_support::LoopbackServer,
    test_store::{Outcome, TestStore},
};

const PATH: &str =
    "20261009/cpython-3.12.12%2B20261009-x86_64-pc-windows-msvc-install_only_stripped.tar.gz";
const KEY: &str = "cpython-3.12.12-windows-x86_64-none";

fn system32() -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32")
}
/// The official layout: `python.exe` beside its DLLs and `Lib`.
fn archive(interpreter: &str) -> Vec<u8> {
    let interpreter = fs::read(system32().join(interpreter)).unwrap();
    let mut tar = tar::Builder::new(Vec::new());
    for (path, body) in [
        ("python.exe", &interpreter[..]),
        ("python312.dll", b"dll"),
        ("Lib/os.py", b"# stdlib"),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, format!("python/{path}"), body)
            .unwrap();
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}
fn plan(bytes: &[u8]) -> PythonPlan {
    let platform = Platform::WindowsX64;
    let sha256 = format!("{:x}", Sha256::digest(bytes));
    let artifact = official_artifact(&Version::new(3, 12, 12), platform, PATH, sha256).unwrap();
    let tool = lock::Tool {
        adapter: "python".into(),
        version: "3.12.12".into(),
        declarations: vec![Declaration {
            file: ".python-version".into(),
            field: None,
            request: Some("3.12".into()),
        }],
        options: BTreeMap::new(),
        installation: Installation::Managed {
            artifacts: BTreeMap::from([(
                platform,
                BTreeMap::from([("distribution".into(), artifact)]),
            )]),
        },
    };
    let lock = Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: BTreeMap::from([("python".into(), tool)]),
    })
    .unwrap();
    PythonPlan::from_lock(&lock, platform).unwrap()
}
async fn install(
    store: &TestStore,
    plan: &PythonPlan,
    upstream: &LoopbackServer,
) -> Result<Outcome, Error> {
    let transport = PythonTransport::loopback_http_for_tests(upstream.origin()).unwrap();
    let desired = [plan.inventory_tool().clone()];
    let prepared = plan
        .prepare(store, &desired, &transport, crate::Preparation::IfNeeded)
        .await?;
    Ok(store.reconcile(&desired, |tool, path| {
        prepared
            .as_ref()
            .ok_or(turborepo_tool_install::Error::InvalidInventory)?
            .stage(tool, path)
    })?)
}
fn names(directory: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[tokio::test(flavor = "current_thread")]
async fn verified_build_runs_in_place_through_the_batch_launcher() {
    // A real PE image stands in for python.exe so the launcher chain runs.
    let bytes = archive("cmd.exe");
    let upstream =
        LoopbackServer::new([(format!("{RELEASES}{PATH}"), bytes.clone())], |_| {}).unwrap();
    let plan = plan(&bytes);
    let repo = tempfile::tempdir().unwrap();
    let store = TestStore::open(repo.path());
    assert_eq!(
        install(&store, &plan, &upstream).await.unwrap(),
        Outcome::Replaced
    );
    let current = store.current().unwrap().unwrap();
    let tree = store.tree_of(plan.inventory_tool());
    assert!(tree.join(KEY).join("Lib").join("os.py").is_file());
    assert!(tree.join(KEY).join("python312.dll").is_file());
    assert_eq!(names(&current.bin), ["python.cmd"]);
    // uv's install-directory scan skips the dot directory, so the key
    // directory stays its only installation.
    let launch = (plan.inventory_tool().executables["python"].as_file())
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    assert_eq!(names(&tree), [launch.clone(), KEY.to_owned()]);
    assert_eq!(
        fs::read_to_string(tree.join(&launch).join("python.cmd")).unwrap(),
        format!("@\"%~dp0..\\{KEY}\\python.exe\" %*\r\n")
    );
    // The bin forwarder and the launcher reach the in-place interpreter and
    // preserve its arguments and exit status.
    let status = Command::new(system32().join("cmd.exe"))
        .args(["/d", "/c", "python /d /c exit 4"])
        .env("PATH", dunce::simplified(&current.bin))
        .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(4));
    assert_eq!(
        install(&store, &plan, &upstream).await.unwrap(),
        Outcome::Unchanged
    );
    assert_eq!(upstream.hits(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn failed_bytecode_compilation_publishes_nothing() {
    // where.exe rejects compileall's arguments.
    let bytes = archive("where.exe");
    let upstream =
        LoopbackServer::new([(format!("{RELEASES}{PATH}"), bytes.clone())], |_| {}).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let store = TestStore::open(repo.path());
    assert!(matches!(
        install(&store, &plan(&bytes), &upstream).await,
        Err(Error::Install(turborepo_tool_install::Error::Io(_)))
    ));
    assert!(store.current().unwrap().is_none());
}

#[test]
fn bytecode_compilation_keeps_only_systemroot() {
    let command = compile_command(Path::new(r"C:\install"), "python.exe", "Lib");
    assert_eq!(command.get_program(), r"C:\install\python.exe");
    assert_eq!(command.get_args().last(), Some(OsStr::new("Lib")));
    let root = std::env::var_os("SYSTEMROOT").unwrap();
    assert_eq!(
        command.get_envs().collect::<Vec<_>>(),
        [(OsStr::new("SYSTEMROOT"), Some(root.as_os_str()))]
    );
}
