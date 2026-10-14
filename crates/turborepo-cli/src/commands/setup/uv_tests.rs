//! Python-only parser → discovery → uv lock writer → provisioning fixtures.
//! Fixture uv binaries are installed but never executed by setup.
use turborepo_setup::{
    lock::{Lock, reconcile},
    uv::UvArtifact,
    uv_resolution::UvTransport,
};

use super::*;

const PYTHON: &str =
    r#"{"futureFlags":{"experimentalSetup":true,"experimentalPythonWorkspaces":true}}"#;
const LOCAL: &[&str] = &["--tools-only", "--no-frozen"];
const REFRESH: &[&str] = &["--tools-only", "--no-frozen", "--update-lock"];

fn fixture(flags: &str, request: &str) -> OwnedSetupFixture {
    let owned = OwnedSetupFixture::new().unwrap();
    owned.init_git().unwrap();
    fs::write(owned.root().join("turbo.json"), flags).unwrap();
    fs::create_dir_all(owned.root().join("apps/web/src")).unwrap();
    declare(&owned, request);
    owned
}
fn declare(owned: &OwnedSetupFixture, request: &str) {
    let pyproject = format!(
        "[project]\nname = \"app\"\nversion = \"0.1.0\"\n[tool.uv]\nrequired-version = \
         {request:?}\n[tool.uv.workspace]\nmembers = []\n"
    );
    fs::write(owned.root().join("pyproject.toml"), pyproject).unwrap();
}
fn upstream(versions: &[&str]) -> LoopbackServer {
    let platform = provision::platform().unwrap();
    let mut index = String::new();
    let mut routes = Vec::new();
    for version in versions {
        let official =
            UvArtifact::for_platform(&semver::Version::parse(version).unwrap(), platform).unwrap();
        let bytes = archive(
            &format!("uv-{}", official.target()),
            &[
                ("uv", "#!/bin/sh\ntouch probe-ran\n"),
                ("uvx", "#!/bin/sh\nexit 1\n"),
            ],
        );
        index += &format!("{}\n", json!({"version": version, "artifacts": []}));
        let release = format!("/github/uv/releases/download/{version}");
        let sums = format!("{:x} *{}\n", Sha256::digest(&bytes), official.filename());
        routes.push((format!("{release}/sha256.sum"), sums.into_bytes()));
        routes.push((format!("{release}/{}", official.filename()), bytes));
    }
    routes.push((
        "/github/versions/main/v1/uv.ndjson".into(),
        index.into_bytes(),
    ));
    LoopbackServer::new(routes, |_| {}).unwrap()
}
fn run(owned: &OwnedSetupFixture, upstream: &LoopbackServer, flags: &[&str]) -> Result<i32, Error> {
    let cwd = owned.root().join("apps/web/src");
    let mut words = vec![
        "turbo".into(),
        "setup".into(),
        "--cwd".into(),
        cwd.clone().into_os_string(),
    ];
    words.extend(flags.iter().map(std::ffi::OsString::from));
    let args = Args::parse_args(words).unwrap();
    let origin = upstream.origin();
    crate::cli::dispatch_setup(&args, |args, setup_args| {
        run_with_policy(
            args,
            setup_args,
            Some(provision::Transports {
                trust_base: Some(owned.home().to_owned()),
                node: turborepo_setup::node_provision::NodeTransport::loopback_http_for_tests(origin)
                    .unwrap(),
                pnpm: turborepo_setup::pnpm_provision::PnpmTransport::loopback_http_for_tests(origin)
                    .unwrap(),
                registry:
                    turborepo_setup::registry_resolution::RegistryTransport::loopback_http_for_tests(
                        origin,
                    )
                    .unwrap(),
                uv: UvTransport::loopback_http_for_tests(origin).unwrap(),
                rustup: turborepo_setup::rustup_provision::RustupTransport::loopback_http_for_tests(origin).unwrap(),
            }),
            || Ok(owned.policy_at(&cwd)?),
        )
    })
    .unwrap()
}
fn locked(owned: &OwnedSetupFixture) -> Option<Lock> {
    Lock::read(owned.root()).unwrap()
}
fn uv_version(owned: &OwnedSetupFixture) -> String {
    locked(owned).unwrap().tools()["uv"].version.clone()
}

#[test]
fn python_only_local_setup_locks_installs_and_repeats_without_traffic() {
    let owned = fixture(PYTHON, ">=0.12, <0.14");
    let upstream = upstream(&["0.12.24", "0.13.0", "0.14.0"]);
    assert_eq!(run(&owned, &upstream, LOCAL).unwrap(), 0);
    let lock = locked(&owned).unwrap();
    assert_eq!(lock.tools().keys().collect::<Vec<_>>(), ["uv"]);
    let uv = &lock.tools()["uv"];
    assert_eq!(uv.version, "0.13.0");
    assert_eq!(
        serde_json::to_value(&uv.declarations).unwrap(),
        json!([{"file":"pyproject.toml","field":"tool.uv.required-version",
            "request":">=0.12, <0.14"}])
    );
    let store = turborepo_tool_install::Store::open(owned.root()).unwrap();
    let current = store.current().unwrap().unwrap();
    drop(store);
    assert_eq!(current.tools.len(), 1);
    assert!(current.bin.join("uv").exists() && current.bin.join("uvx").exists());
    assert_eq!(upstream.hits(), 3);
    let bytes = fs::read(owned.root().join("turbo.lock")).unwrap();
    for flags in [LOCAL, FROZEN] {
        assert_eq!(run(&owned, &upstream, flags).unwrap(), 0);
    }
    assert_eq!(upstream.hits(), 3);
    assert_eq!(fs::read(owned.root().join("turbo.lock")).unwrap(), bytes);
    assert!(matches!(
        run(&owned, &upstream, &["--check", "--tools-only"]),
        Err(Error::Unsupported(_))
    ));
    assert!(!owned.root().join("probe-ran").exists());
}

#[test]
fn declaration_changes_update_locally_and_frozen_names_uv() {
    let owned = fixture(PYTHON, ">=0.12");
    let upstream = upstream(&["0.12.24", "0.13.0"]);
    assert_eq!(run(&owned, &upstream, LOCAL).unwrap(), 0);
    assert_eq!(uv_version(&owned), "0.13.0");
    declare(&owned, "==0.12.*");
    let bytes = fs::read(owned.root().join("turbo.lock")).unwrap();
    match run(&owned, &upstream, FROZEN) {
        Err(Error::Reconcile(reconcile::Error::Drift(ids))) => {
            assert_eq!(ids.into_iter().collect::<Vec<_>>(), ["uv"]);
        }
        other => panic!("expected named uv drift, got {other:?}"),
    }
    assert_eq!(fs::read(owned.root().join("turbo.lock")).unwrap(), bytes);
    assert_eq!(run(&owned, &upstream, LOCAL).unwrap(), 0);
    assert_eq!(uv_version(&owned), "0.12.24");
    // With no remaining declaration, local setup fails closed instead of
    // publishing an empty lock.
    fs::write(
        owned.root().join("pyproject.toml"),
        "[project]\nname = \"app\"\n[tool.uv.workspace]\nmembers = []\n",
    )
    .unwrap();
    assert!(run(&owned, &upstream, FROZEN).is_err());
    assert!(run(&owned, &upstream, LOCAL).is_err());
    assert_eq!(uv_version(&owned), "0.12.24");
}

#[test]
fn floating_requests_refresh_only_with_update_lock() {
    let owned = fixture(PYTHON, ">=0.12");
    assert_eq!(run(&owned, &upstream(&["0.12.24"]), LOCAL).unwrap(), 0);
    let newer = upstream(&["0.12.24", "0.13.0"]);
    assert_eq!(run(&owned, &newer, LOCAL).unwrap(), 0);
    assert_eq!(uv_version(&owned), "0.12.24");
    assert_eq!(newer.hits(), 0);
    assert_eq!(run(&owned, &newer, REFRESH).unwrap(), 0);
    assert_eq!(uv_version(&owned), "0.13.0");
}

#[test]
fn disabled_python_flag_ignores_uv_declarations() {
    let owned = fixture(ENABLED, ">=0.12");
    let upstream = upstream(&["0.13.0"]);
    let before = publication_guards::state(owned.root());
    assert!(matches!(
        run(&owned, &upstream, LOCAL),
        Err(Error::Unsupported("a native Node declaration is required"))
    ));
    assert_eq!(publication_guards::state(owned.root()), before);
    assert_eq!(upstream.hits(), 0);
    // A uv-only repository with Python enabled but no request has nothing to lock.
    let owned = fixture(PYTHON, ">=0.12");
    fs::write(
        owned.root().join("pyproject.toml"),
        "[tool.uv.workspace]\nmembers = []\n",
    )
    .unwrap();
    assert!(matches!(
        run(&owned, &upstream, LOCAL),
        Err(Error::Unsupported(
            "a native Node or uv declaration is required"
        ))
    ));
    assert!(locked(&owned).is_none());
}
