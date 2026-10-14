//! Python-only parser → discovery → uv lock writer → provisioning fixtures.
//! Fixture uv binaries are installed but never executed by setup.
use turborepo_setup::{lock::Lock, uv::UvArtifact, uv_resolution::UvTransport};

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
    upstream_with(versions, "#!/bin/sh\ntouch probe-ran\n", Vec::new())
}
fn upstream_with(
    versions: &[&str],
    uv: &str,
    mut routes: Vec<(String, Vec<u8>)>,
) -> LoopbackServer {
    let platform = provision::platform().unwrap();
    let mut index = String::new();
    for version in versions {
        let official =
            UvArtifact::for_platform(&semver::Version::parse(version).unwrap(), platform).unwrap();
        let bytes = archive(
            &format!("uv-{}", official.target()),
            &[("uv", uv), ("uvx", "#!/bin/sh\nexit 1\n")],
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
                python: turborepo_setup::python_resolution::PythonTransport::loopback_http_for_tests(origin).unwrap(),
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
        Err(Error::LockDiagnostics(lock_diagnostics::Error::Drift(locations))) => {
            assert_eq!(locations, "uv: pyproject.toml (tool.uv.required-version)");
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

/// Host CPython builds served by the fixture; the fixture uv lists them.
fn python_upstream() -> LoopbackServer {
    let host = match provision::platform().unwrap() {
        turborepo_setup::lock::Platform::MacosArm64 => ("macos", "aarch64", "none"),
        turborepo_setup::lock::Platform::MacosX64 => ("macos", "x86_64", "none"),
        turborepo_setup::lock::Platform::LinuxX64Gnu => ("linux", "x86_64", "gnu"),
        _ => ("linux", "aarch64", "gnu"),
    };
    let releases = "/github/python-build-standalone/releases/download/20260211";
    let entry = |version: &str, (os, arch, libc): (&str, &str, &str), filename: &str| {
        json!({"key":format!("cpython-{version}-{os}-{arch}-{libc}"),
            "version":version,"os":os,"arch":arch,"libc":libc,"implementation":"cpython",
            "variant":"default","url":format!("https://releases.astral.sh{releases}/{}",
                filename.replace('+', "%2B"))})
    };
    let (mut listing, mut sums, mut routes) = (Vec::new(), String::new(), Vec::new());
    for (version, minor) in [("3.12.12", "3.12"), ("3.13.1", "3.13")] {
        let filename = format!("cpython-{version}+20260211-host-install_only_stripped.tar.gz");
        let interpreter = "#!/bin/sh\necho managed-python\n";
        let bytes = archive(
            "python",
            &[
                (&format!("bin/python{minor}"), interpreter),
                ("bin/python3", interpreter),
            ],
        );
        sums += &format!("{:x}  {filename}\n", Sha256::digest(&bytes));
        listing.push(entry(version, host, &filename));
        // Like real `--all-arches` output, list both Linux architectures;
        // only the host build is ever downloaded.
        for linux in [("linux", "x86_64", "gnu"), ("linux", "aarch64", "gnu")] {
            if linux != host {
                let peer = format!("cpython-{version}+20260211-{}-install_only.tar.gz", linux.1);
                sums += &format!("{:064x}  {peer}\n", 1);
                listing.push(entry(version, linux, &peer));
            }
        }
        routes.push((
            format!("{releases}/{}", filename.replace('+', "%2B")),
            bytes,
        ));
    }
    routes.push((format!("{releases}/SHA256SUMS"), sums.into_bytes()));
    let uv = format!(
        "#!/bin/sh\nif [ \"$1\" = python ]; then cat <<'EOF'\n{}\nEOF\nelse echo \
         \"$UV_PYTHON_INSTALL_DIR|$UV_PYTHON_PREFERENCE|$*\"; fi\n",
        Value::Array(listing)
    );
    upstream_with(&["0.13.0"], &uv, routes)
}

#[test]
fn python_and_uv_lock_together_and_launchers_use_only_managed_python() {
    let owned = fixture(PYTHON, ">=0.13");
    fs::write(owned.root().join(".python-version"), "3.12\n").unwrap();
    let upstream = python_upstream();
    assert_eq!(run(&owned, &upstream, LOCAL).unwrap(), 0);
    let lock = locked(&owned).unwrap();
    assert_eq!(lock.tools().keys().collect::<Vec<_>>(), ["python", "uv"]);
    assert_eq!(lock.tools()["python"].version, "3.12.12");
    let store = turborepo_tool_install::Store::open(owned.root()).unwrap();
    let current = store.current().unwrap().unwrap();
    drop(store);
    let tools = fs::canonicalize(current.bin.join("../tools")).unwrap();
    let output = std::process::Command::new(current.bin.join("uv"))
        .args(["run", "app"])
        .env_clear()
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        format!("{}|only-managed|run app", tools.join("python").display())
    );
    assert!(current.bin.join("python3").exists());
    // Resolution lists through a verified scratch uv; provisioning installs both.
    let hits = upstream.hits();
    for flags in [LOCAL, FROZEN] {
        assert_eq!(run(&owned, &upstream, flags).unwrap(), 0);
    }
    assert_eq!(upstream.hits(), hits);
    let committed = fs::read(owned.root().join("turbo.lock")).unwrap();
    fs::write(owned.root().join(".python-version"), "3.13\n").unwrap();
    match run(&owned, &upstream, FROZEN) {
        Err(Error::LockDiagnostics(lock_diagnostics::Error::Drift(locations))) => {
            assert_eq!(locations, "python: .python-version");
        }
        other => panic!("expected named Python drift, got {other:?}"),
    }
    assert_eq!(upstream.hits(), hits);
    assert_eq!(
        fs::read(owned.root().join("turbo.lock")).unwrap(),
        committed
    );
    assert_eq!(run(&owned, &upstream, LOCAL).unwrap(), 0);
    let lock = locked(&owned).unwrap();
    assert_eq!(lock.tools()["python"].version, "3.13.1");
    assert_eq!(uv_version(&owned), "0.13.0");
}
