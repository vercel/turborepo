#![allow(clippy::unwrap_used)]

use std::{collections::BTreeMap, io::Write, process::Command};

use semver::Version;
use sha2::{Digest, Sha256};
use turborepo_tool_install::Outcome;

use super::*;
use crate::{
    lock::{Declaration, Document},
    python_resolution::{RELEASES, official_artifact},
    source_policy::test_support::LoopbackServer,
};

const PATH: &str =
    "20260211/cpython-3.12.12%2B20260211-x86_64-unknown-linux-gnu-install_only_stripped.tar.gz";

fn archive(files: &[(&str, &str)]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, body) in files {
        let mut header = tar::Header::new_gnu();
        // Like the official builds, rename in place: the old tail follows the NUL.
        header.set_path(format!("python/install/{path}")).unwrap();
        header.set_size(body.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, format!("python/{path}"), body.as_bytes())
            .unwrap();
    }
    let mut link = tar::Header::new_gnu();
    link.set_entry_type(tar::EntryType::Symlink);
    link.set_size(0);
    link.set_mode(0o777);
    link.set_link_name("python3.12").unwrap();
    link.set_cksum();
    tar.append_data(&mut link, "python/bin/python3", &[][..])
        .unwrap();
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}
/// Records how staging compiles bytecode, where real CPython would write it.
const INTERPRETER: &str = "#!/bin/sh\nif [ \"$3 $4\" = \"-m compileall\" ]; then\n  mkdir \
                           lib/python3.12/__pycache__ && echo \"${PWD##*/} ${HOME-unset} $*\" > \
                           lib/python3.12/__pycache__/os.pyc\n  exit\nfi\necho \"cpython $0 $*\"\n";
const PYC: &str = "lib/python3.12/__pycache__/os.pyc";

fn good() -> Vec<u8> {
    archive(&[
        ("bin/python3.12", INTERPRETER),
        ("lib/python3.12/os.py", "# stdlib resource"),
    ])
}
fn document(platform: Platform, bytes: &[u8]) -> Document {
    let artifact = official_artifact(
        &Version::new(3, 12, 12),
        platform,
        PATH,
        format!("{:x}", Sha256::digest(bytes)),
    )
    .unwrap();
    Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: BTreeMap::from([(
            "python".into(),
            lock::Tool {
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
            },
        )]),
    }
}
fn plan(document: Document, platform: Platform) -> Result<PythonPlan, Error> {
    PythonPlan::from_lock(&Lock::new(document).unwrap(), platform)
}
async fn install(
    store: &mut Store,
    plan: &PythonPlan,
    transport: &PythonTransport,
) -> Result<Outcome, Error> {
    let desired = [plan.inventory_tool().clone()];
    let prepared = plan
        .prepare(store, &desired, transport, crate::Preparation::IfNeeded)
        .await?;
    Ok(store.reconcile(&desired, |tool, path| {
        prepared
            .as_ref()
            .ok_or(turborepo_tool_install::Error::InvalidInventory)?
            .stage(tool, path)
    })?)
}

#[tokio::test(flavor = "current_thread")]
async fn verified_build_is_staged_at_the_uv_key_with_python_shims() {
    let platform = Platform::LinuxX64Gnu;
    let bytes = good();
    let upstream =
        LoopbackServer::new([(format!("{RELEASES}{PATH}"), bytes.clone())], |_| {}).unwrap();
    let transport = PythonTransport::loopback_http_for_tests(upstream.origin()).unwrap();
    let plan = plan(document(platform, &bytes), platform).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    assert_eq!(
        install(&mut store, &plan, &transport).await.unwrap(),
        Outcome::Replaced
    );
    let current = store.current().unwrap().unwrap();
    let tree = current
        .bin
        .join("../tools/python/cpython-3.12.12-linux-x86_64-gnu");
    assert!(tree.join("lib/python3.12/os.py").is_file());
    assert_eq!(
        fs::read_to_string(tree.join(PYC)).unwrap(),
        "cpython-3.12.12-linux-x86_64-gnu unset -I -B -m compileall -q -f -j 0 \
         --invalidation-mode unchecked-hash lib/python3.12\n"
    );
    for name in ["python", "python3"] {
        let output = Command::new(current.bin.join(name))
            .arg("-V")
            .env_clear()
            .output()
            .unwrap();
        let expected = format!("cpython {} -V\n", current.bin.join(name).display());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    }
    assert_eq!(
        install(&mut store, &plan, &transport).await.unwrap(),
        Outcome::Unchanged
    );
    assert_eq!(upstream.hits(), 1);
    // Bytecode is hashed like every other managed byte.
    fs::write(tree.join(PYC), b"stale").unwrap();
    assert_eq!(
        install(&mut store, &plan, &transport).await.unwrap(),
        Outcome::Replaced
    );
    assert_eq!(upstream.hits(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn failed_bytecode_compilation_publishes_nothing() {
    let platform = Platform::LinuxX64Gnu;
    let bytes = archive(&[("bin/python3.12", "#!/bin/sh\nexit 3\n")]);
    let upstream =
        LoopbackServer::new([(format!("{RELEASES}{PATH}"), bytes.clone())], |_| {}).unwrap();
    let transport = PythonTransport::loopback_http_for_tests(upstream.origin()).unwrap();
    let plan = plan(document(platform, &bytes), platform).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    assert!(matches!(
        install(&mut store, &plan, &transport).await,
        Err(Error::Install(turborepo_tool_install::Error::Io(_)))
    ));
    assert!(store.current().unwrap().is_none());
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "current_thread")]
async fn linux_terminfo_case_siblings_install_as_distinct_terminals() {
    let platform = Platform::LinuxX64Gnu;
    let bytes = archive(&[
        ("bin/python3.12", "#!/bin/sh\n"),
        ("share/terminfo/E/Eterm", "Eterm"),
        ("share/terminfo/e/eterm", "eterm"),
    ]);
    let upstream =
        LoopbackServer::new([(format!("{RELEASES}{PATH}"), bytes.clone())], |_| {}).unwrap();
    let transport = PythonTransport::loopback_http_for_tests(upstream.origin()).unwrap();
    let plan = plan(document(platform, &bytes), platform).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    install(&mut store, &plan, &transport).await.unwrap();
    let terminfo = store
        .current()
        .unwrap()
        .unwrap()
        .bin
        .join("../tools/python/cpython-3.12.12-linux-x86_64-gnu/share/terminfo");
    for name in ["E/Eterm", "e/eterm"] {
        let body = fs::read_to_string(terminfo.join(name)).unwrap();
        assert_eq!(body, name[2..]);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn tampered_builds_never_publish() {
    let platform = Platform::MacosArm64;
    let tampered = archive(&[("bin/python3.12", "tampered")]);
    let upstream = LoopbackServer::new([(format!("{RELEASES}{PATH}"), tampered)], |_| {}).unwrap();
    let plan = plan(document(platform, &good()), platform).unwrap();
    let transport = PythonTransport::loopback_http_for_tests(upstream.origin()).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    assert!(matches!(
        install(&mut store, &plan, &transport).await,
        Err(Error::Download(turborepo_download::Error::DigestMismatch))
    ));
    assert!(store.current().unwrap().is_none());
}

#[test]
fn plans_accept_only_official_builds_on_qualified_targets() {
    let bytes = good();
    for platform in [Platform::LinuxX64Musl, Platform::WindowsX64] {
        assert!(matches!(
            plan(document(platform, &bytes), platform),
            Err(Error::UnsupportedTarget)
        ));
    }
    assert!(matches!(
        plan(
            document(Platform::LinuxX64Gnu, &bytes),
            Platform::MacosArm64
        ),
        Err(Error::MissingTarget)
    ));
    let edits: [fn(&mut lock::Artifact); 2] = [
        |a| a.destination = Some("cpython-3.12.11-linux-x86_64-gnu".into()),
        |a| drop(a.executables.insert("pip".into(), "bin/pip".into())),
    ];
    for edit in edits {
        let mut document = document(Platform::LinuxX64Gnu, &bytes);
        let Installation::Managed { artifacts } =
            &mut document.tools.get_mut("python").unwrap().installation
        else {
            unreachable!()
        };
        edit(
            artifacts
                .get_mut(&Platform::LinuxX64Gnu)
                .unwrap()
                .get_mut("distribution")
                .unwrap(),
        );
        assert!(matches!(
            plan(document, Platform::LinuxX64Gnu),
            Err(Error::InvalidLock)
        ));
    }
}
