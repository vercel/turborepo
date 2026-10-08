//! Real CLI consumers of the dev-dependency seam. The executor is still the
//! explicit NotImplemented gate on main; these tests do not qualify
//! provisioning or native/packaged-wrapper process parity.

use std::{fs, time::Duration};

use turborepo_download::{DownloadClient, ExpectedSha256, Limits};
use turborepo_setup::{
    lock::{Lock, Platform},
    node_provision::{NodePlan, NodeTransport},
    npm_provision::NpmTransport,
    pnpm_provision::PnpmTransport,
    source_policy::{ConfigOwner, Error as PolicyError},
    test_support::OwnedSetupFixture,
};

use super::*;
use crate::{cli::Command, commands::setup};

type TestResult = Result<(), Box<dyn std::error::Error>>;
const ENABLED: &str = r#"{"futureFlags":{"experimentalSetup":true}}"#;
const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn fixture() -> Result<OwnedSetupFixture, Box<dyn std::error::Error>> {
    let fixture = OwnedSetupFixture::new()?;
    fs::create_dir(fixture.root().join(".git"))?;
    fs::create_dir_all(fixture.root().join("apps/web/src"))?;
    fs::write(fixture.root().join("turbo.json"), ENABLED)?;
    Ok(fixture)
}

fn args(fixture: &OwnedSetupFixture) -> Result<Args, Box<dyn std::error::Error>> {
    Ok(Args::parse_args(vec![
        "turbo".into(),
        "setup".into(),
        "--cwd".into(),
        fixture.root().join("apps/web/src").into_os_string(),
        "--frozen".into(),
        "--tools-only".into(),
    ])?)
}

#[cfg(unix)]
#[test]
fn owned_policy_reaches_actual_parser_root_and_executor_without_success_or_writes() -> TestResult {
    let f = fixture()?;
    let args = args(&f)?;
    let Some(Command::Setup { setup_args }) = &args.command else {
        return Err("not setup".into());
    };
    let discovery = Discovery::capture(&args)?;
    assert_eq!(discovery.snapshot_root()?.as_std_path(), f.root());
    assert!(discovery.revalidate()?);
    assert_eq!(
        f.policy_at(discovery.root_path().as_std_path())?.registry(),
        "https://registry.npmjs.org"
    );
    let request = setup::SetupRequest::new(setup_args, true)?;
    assert_eq!(request.lock, setup::LockMode::Frozen);
    assert!(request.tools_only);
    assert!(matches!(
        setup::execute(request),
        Err(setup::Error::NotImplemented)
    ));
    assert!(matches!(
        setup::run(&args, setup_args),
        Err(setup::Error::NotImplemented)
    ));
    fs::write(f.home().join(".npmrc"), "")?;
    assert_eq!(
        f.policy().err(),
        Some(PolicyError::Configured(ConfigOwner::User))
    );
    assert!(!f.root().join(".turbo").exists());
    assert!(!f.root().join("turbo.lock").exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn owned_nested_config_workspace_and_git_drift_preserve_discovery_guard() -> TestResult {
    for (marker, body) in [
        ("turbo.json", ENABLED),
        ("turbo.jsonc", ENABLED),
        ("pnpm-workspace.yaml", "packages: []\n"),
        ("package.json", r#"{"workspaces":[]}"#),
        (".git", "gitdir: /not-probed/worktrees/web\n"),
    ] {
        let f = fixture()?;
        let args = args(&f)?;
        let d = Discovery::capture(&args)?;
        let nested = f.root().join("apps/web");
        assert!(f.policy_at(&nested)?.registry().starts_with("https://"));
        fs::write(nested.join(marker), body)?;
        match d.revalidate() {
            Ok(false) => assert_eq!(marker, ".git"),
            Err(Error::Ambiguous { .. }) => assert!(marker.starts_with("turbo.json")),
            Err(Error::Nested { .. }) => {
                assert!(matches!(marker, "package.json" | "pnpm-workspace.yaml"))
            }
            other => return Err(format!("unexpected drift result: {other:?}").into()),
        }
        assert_eq!(d.root_path().as_std_path(), f.root());
        assert!(!f.root().join(".turbo").exists());
    }
    let f = fixture()?;
    let args = args(&f)?;
    let d = Discovery::capture(&args)?;
    fs::write(f.root().join("turbo.json"), r#"{"setup":{}}"#)?;
    assert_eq!(
        f.policy().err(),
        Some(PolicyError::Configured(ConfigOwner::RootPolicy))
    );
    assert!(matches!(d.revalidate(), Err(Error::TurboJson(_))));
    Ok(())
}

fn locked_node() -> Result<Lock, Box<dyn std::error::Error>> {
    Ok(Lock::parse(&serde_json::to_vec(&serde_json::json!({
        "schemaVersion":0,"tools":{"node":{
            "adapter":"node","version":"24.0.0",
            "declarations":[{"file":".nvmrc","request":"24.0.0"}],
            "installation":{"kind":"managed","artifacts":{"linux-x64-gnu":{"distribution":{
                "url":"https://nodejs.org/dist/v24.0.0/node-v24.0.0-linux-x64.tar.gz",
                "sha256":ABC_SHA256,"format":"tar-gz","rootPrefix":"node-v24.0.0-linux-x64",
                "executables":{"node":"bin/node"}
            }}}}
        }}
    }))?)?)
}

#[cfg(unix)]
#[tokio::test]
async fn owned_loopback_download_and_adapter_preserve_official_lock_provenance() -> TestResult {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let f = fixture()?;
    let _policy = f.policy()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let client = DownloadClient::loopback_http_for_tests(&origin)?;
    let node = NodeTransport::loopback_http_for_tests(&origin)?;
    let _pnpm = PnpmTransport::loopback_http_for_tests(&origin)?;
    let _npm = NpmTransport::loopback_http_for_tests(&origin)?;
    let lock = locked_node()?;
    let before = lock.clone();
    let plan = NodePlan::from_lock(&lock, Platform::LinuxX64Gnu)?;
    let operation = async {
        let bytes = client
            .download_verified(
                &format!("{origin}/artifact"),
                ExpectedSha256::from_hex(ABC_SHA256)?,
                Limits::new(3, Duration::from_secs(5))?,
            )
            .await?;
        assert_eq!(bytes.as_bytes(), b"abc");
        // Known digest, deliberately malformed archive: the real adapter must
        // reject extraction rather than install or manufacture readiness.
        assert!(matches!(
            plan.download(&node).await,
            Err(turborepo_setup::node_provision::Error::Archive(_))
        ));
        TestResult::Ok(())
    };
    let server = async {
        for path in ["/artifact", "/dist/v24.0.0/node-v24.0.0-linux-x64.tar.gz"] {
            let (mut stream, _) = listener.accept().await?;
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let n = stream.read(&mut buffer).await?;
                if n == 0 || request.len() + n > 8192 {
                    return Err("invalid fixture request".into());
                }
                request.extend_from_slice(&buffer[..n]);
            }
            assert!(
                std::str::from_utf8(&request)?.starts_with(&format!("GET {path} HTTP/1.1\r\n"))
            );
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc")
                .await?;
        }
        TestResult::Ok(())
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::try_join!(operation, server)
    })
    .await??;
    assert_eq!(lock, before);
    assert!(!f.root().join(".turbo").exists());
    assert!(matches!(
        client
            .read_metadata(
                "https://nodejs.org/dist/index.json",
                Limits::new(3, Duration::from_secs(1))?
            )
            .await,
        Err(turborepo_download::Error::UnapprovedOrigin)
    ));
    Ok(())
}

#[test]
fn cross_crate_loopback_seam_rejects_nonliteral_and_nonorigin_inputs() {
    for origin in [
        "http://localhost:1234",
        "http://192.0.2.1:1234",
        "https://127.0.0.1:1234",
        "http://user:private-secret@127.0.0.1:1234",
        "http://127.0.0.1:1234/path",
        "http://127.0.0.1:1234/?secret=x",
    ] {
        assert!(DownloadClient::loopback_http_for_tests(origin).is_err());
        assert!(NodeTransport::loopback_http_for_tests(origin).is_err());
        assert!(PnpmTransport::loopback_http_for_tests(origin).is_err());
        assert!(NpmTransport::loopback_http_for_tests(origin).is_err());
    }
    for origin in ["http://127.0.0.2:1234", "http://[::1]:1234/"] {
        assert!(DownloadClient::loopback_http_for_tests(origin).is_ok());
        assert!(NodeTransport::loopback_http_for_tests(origin).is_ok());
        assert!(PnpmTransport::loopback_http_for_tests(origin).is_ok());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn unsupported_unix_artifact_targets_fail_before_loopback_traffic_or_storage() -> TestResult {
    let f = fixture()?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let _transport =
        NodeTransport::loopback_http_for_tests(&format!("http://{}", listener.local_addr()?))?;
    let lock = locked_node()?;
    for target in [
        Platform::LinuxX64Musl,
        Platform::LinuxArm64Musl,
        Platform::Any,
    ] {
        assert!(matches!(
            NodePlan::from_lock(&lock, target),
            Err(turborepo_setup::node_provision::Error::UnsupportedTarget)
        ));
    }
    let args = args(&f)?;
    let Some(Command::Setup { setup_args }) = &args.command else {
        return Err("not setup".into());
    };
    assert!(matches!(
        setup::run(&args, setup_args),
        Err(setup::Error::NotImplemented)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
    assert!(!f.root().join(".turbo").exists());
    assert!(!f.root().join("turbo.lock").exists());
    Ok(())
}
