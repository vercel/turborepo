//! Python-only parser → discovery → uv lock writer → provisioning fixtures.
//! Fixture uv binaries are installed but never executed by setup.
use turborepo_setup::{lock::Lock, uv::UvArtifact, uv_resolution::UvTransport};

use super::*;

const PYTHON: &str =
    r#"{"futureFlags":{"experimentalSetup":true,"experimentalPythonWorkspaces":true}}"#;
const LOCAL: &[&str] = &["--tools-only", "--no-frozen"];
const REFRESH: &[&str] = &["--tools-only", "--no-frozen", "--update-lock"];
const CHECK: &[&str] = &["--check", "--tools-only"];

fn fixture(flags: &str, request: &str) -> OwnedSetupFixture {
    let owned = OwnedSetupFixture::new().unwrap();
    owned.fake_git().unwrap();
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
                tool_store: Some(owned.home().to_owned()),
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
                bun: turborepo_setup::bun_provision::BunTransport::loopback_http_for_tests(origin).unwrap(),
                python: turborepo_setup::python_resolution::PythonTransport::loopback_http_for_tests(origin).unwrap(),
                artifact: None,
                go: turborepo_setup::go_resolution::GoTransport::loopback_http_for_tests(origin)
                    .unwrap(),
            }),
            turborepo_setup::git::FakeGit::shared(),
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
    assert_eq!(fs::read(owned.root().join("turbo.lock")).unwrap(), bytes);
    let before = publication_guards::state(owned.root());
    assert_eq!(run(&owned, &upstream, CHECK).unwrap(), 0);
    assert_eq!(publication_guards::state(owned.root()), before);
    assert_eq!(upstream.hits(), 3);
    assert!(!owned.root().join("probe-ran").exists());
}

#[test]
fn telemetry_counts_downloaded_uv_bytes() {
    let owned = fixture(PYTHON, ">=0.12, <0.14");
    let upstream = upstream(&["0.12.24", "0.13.0", "0.14.0"]);
    let platform = provision::platform().unwrap();
    let official =
        UvArtifact::for_platform(&semver::Version::parse("0.13.0").unwrap(), platform).unwrap();
    let artifact = archive(
        &format!("uv-{}", official.target()),
        &[
            ("uv", "#!/bin/sh\ntouch probe-ran\n"),
            ("uvx", "#!/bin/sh\nexit 1\n"),
        ],
    )
    .len() as u64;
    let cwd = owned.root().join("apps/web/src");
    let report = |flags: &[&str]| {
        let mut words = vec![
            "turbo".into(),
            "setup".into(),
            "--cwd".into(),
            cwd.clone().into_os_string(),
        ];
        words.extend(flags.iter().map(std::ffi::OsString::from));
        let args = Args::parse_args(words).unwrap();
        let origin = upstream.origin();
        let mut report = telemetry::Report::default();
        let code = crate::cli::dispatch_setup(&args, |args, setup_args| {
            let transports = provision::Transports {
                tool_store: Some(owned.home().to_owned()),
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
                rustup:
                    turborepo_setup::rustup_provision::RustupTransport::loopback_http_for_tests(
                        origin,
                    )
                    .unwrap(),
                bun: turborepo_setup::bun_provision::BunTransport::loopback_http_for_tests(origin)
                    .unwrap(),
                python:
                    turborepo_setup::python_resolution::PythonTransport::loopback_http_for_tests(
                        origin,
                    )
                    .unwrap(),
                artifact: None,
                go: turborepo_setup::go_resolution::GoTransport::loopback_http_for_tests(origin)
                    .unwrap(),
            };
            run_reporting(args, setup_args, Some(transports), turborepo_setup::git::FakeGit::shared(), &mut report, &output::Console::capture(output::Mode::Plain).0, || {
                Ok(owned.policy_at(&cwd)?)
            })
            .map(|completed| completed.exit_code())
        })
        .unwrap()
        .unwrap();
        assert_eq!(code, 0);
        (
            report.downloaded_bytes,
            report.tools_installed,
            report.tools_reused,
        )
    };
    // A uv-only repository transfers its verified archive once, then reuses it.
    assert_eq!(report(LOCAL), (artifact, 1, 0));
    assert_eq!(report(LOCAL), (0, 0, 1));
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
fn managed_run_keeps_uv_on_the_locked_python_and_follows_the_workspace_flag() {
    let owned = fixture(PYTHON, ">=0.13");
    fs::write(owned.root().join(".python-version"), "3.12\n").unwrap();
    assert_eq!(run(&owned, &python_upstream(), LOCAL).unwrap(), 0);
    let (env, bin) = run_overrides(&owned).unwrap();
    assert!(bin.join("uv").exists() && bin.join("python3").exists());
    assert_eq!(env["UV_PYTHON_DOWNLOADS"], "never");
    assert_eq!(env["UV_PYTHON_PREFERENCE"], "only-managed");
    assert!(env["UV_PYTHON"].starts_with("cpython-3.12.12-"), "{env:?}");
    // Without the flag setup never reads uv declarations, so run rejects them.
    fs::write(owned.root().join("turbo.json"), ENABLED).unwrap();
    let error = run_overrides(&owned).unwrap_err();
    assert!(error.contains("experimentalPythonWorkspaces"), "{error}");
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
    // Resolution lists through a verified scratch uv whose archive provisioning
    // then reuses, so it is fetched once.
    let archive = UvArtifact::for_platform(
        &semver::Version::new(0, 13, 0),
        provision::platform().unwrap(),
    )
    .unwrap()
    .filename();
    let fetched = |name: &str| {
        let requests = upstream.requests();
        requests.iter().filter(|path| path.ends_with(name)).count()
    };
    assert_eq!(fetched(&archive), 1);
    let hits = upstream.hits();
    for flags in [LOCAL, FROZEN, CHECK] {
        assert_eq!(run(&owned, &upstream, flags).unwrap(), 0);
    }
    assert_eq!(upstream.hits(), hits);
    let committed = fs::read(owned.root().join("turbo.lock")).unwrap();
    // Readiness follows the same workspace-flag gating as setup.
    fs::write(owned.root().join("turbo.json"), ENABLED).unwrap();
    assert!(matches!(
        run(&owned, &upstream, CHECK),
        Err(Error::Activation(
            turborepo_setup::activation::Error::UnsupportedAdapter
        ))
    ));
    fs::write(owned.root().join("turbo.json"), PYTHON).unwrap();
    fs::write(owned.root().join(".python-version"), "3.13\n").unwrap();
    assert!(matches!(
        run(&owned, &upstream, CHECK),
        Err(Error::Activation(
            turborepo_setup::activation::Error::DeclarationDrift
        ))
    ));
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

const UV_LOCK: &[u8] = b"version = 1\n";

#[test]
fn locked_uv_workspaces_sync_with_the_managed_uv_and_its_environment() {
    let owned = fixture(PYTHON, ">=0.12");
    let root = owned.root();
    fs::write(root.join("uv.lock"), UV_LOCK).unwrap();
    // Records its arguments and an activation variable, then syncs.
    let uv = "#!/bin/sh\necho \"$* $UV_PYTHON_DOWNLOADS\" >> uv-args\n[ -f stale ] && exit \
              2\nmkdir -p .venv\n";
    let u = upstream_with(&["0.12.24"], uv, Vec::new());
    assert_eq!(run(&owned, &u, &["--no-frozen"]).unwrap(), 0);
    assert!(root.join(".venv").exists());
    fs::remove_dir_all(root.join(".venv")).unwrap();
    assert_eq!(run(&owned, &u, &["--frozen"]).unwrap(), 0);
    assert!(root.join(".venv").exists());
    // uv's refusal of a stale lock passes its exit status through, and
    // tools-only setup never syncs.
    fs::write(root.join("stale"), "").unwrap();
    assert_eq!(run(&owned, &u, &["--frozen"]).unwrap(), 2);
    assert_eq!(run(&owned, &u, &["--frozen", "--tools-only"]).unwrap(), 0);
    let args = fs::read_to_string(root.join("uv-args")).unwrap();
    assert_eq!(
        args.lines().collect::<Vec<_>>(),
        [
            "sync --locked --all-packages never",
            "sync --locked --all-packages never",
            "sync --locked --all-packages never",
        ]
    );
    assert_eq!(fs::read(root.join("uv.lock")).unwrap(), UV_LOCK);
    assert!(!root.join("probe-ran").exists());
}

#[test]
fn python_dependencies_need_a_committed_uv_lock_before_any_download() {
    let owned = fixture(PYTHON, ">=0.12");
    let u = upstream(&["0.12.24"]);
    for flags in [&["--no-frozen"][..], &["--plan"]] {
        let error = run(&owned, &u, flags).unwrap_err().to_string();
        assert!(error.contains("uv.lock is missing"), "{error}");
    }
    assert_eq!(u.hits(), 0);
    assert!(!owned.root().join("uv.lock").exists());
    assert!(!owned.root().join("turbo.lock").exists());
}
