//! Native Windows x64 uv and CPython through the real CLI path with owned
//! fixtures: a committed lock with `windows-x64` entries, `.exe` and `.cmd`
//! exports, activation variables, check, repeat, and `uv sync --locked`
//! through the managed `uv.exe`.
use turborepo_setup::uv::UvArtifact;

use super::*;

const PYTHON: &str =
    r#"{"futureFlags":{"experimentalSetup":true,"experimentalPythonWorkspaces":true}}"#;
const BUILD: &str = "/github/python-build-standalone/releases/download/20261009/cpython-3.12.12%\
                     2B20261009-x86_64-pc-windows-msvc-install_only_stripped.tar.gz";
const KEY: &str = "cpython-3.12.12-windows-x86_64-none";

/// The official flat zip, with real PE images standing in for the binaries.
fn uv_zip(binary: &str) -> Vec<u8> {
    let binary = fs::read(system32().join(binary)).unwrap();
    turborepo_setup::test_support::stored_zip_entries(&[
        ("uv.exe", &binary[..]),
        ("uvw.exe", b"windowed uv"),
        ("uvx.exe", &binary[..]),
    ])
}
fn python_build() -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    let interpreter = fs::read(system32().join("cmd.exe")).unwrap();
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

struct Repo {
    owned: OwnedSetupFixture,
    upstream: LoopbackServer,
}
impl Repo {
    /// A Python-only repository whose committed lock selects uv and CPython.
    fn new(uv_binary: &str) -> Self {
        let owned = OwnedSetupFixture::new().unwrap();
        let root = owned.root();
        owned.fake_git().unwrap();
        fs::write(root.join("turbo.json"), PYTHON).unwrap();
        fs::write(
            root.join("pyproject.toml"),
            "[project]\nname = \"app\"\nversion = \"0.1.0\"\n[tool.uv]\nrequired-version = \
             \">=0.13\"\n[tool.uv.workspace]\nmembers = []\n",
        )
        .unwrap();
        fs::write(root.join(".python-version"), "3.12\n").unwrap();
        fs::create_dir_all(root.join("apps/web/src")).unwrap();
        let (uv, python) = (uv_zip(uv_binary), python_build());
        let official = UvArtifact::for_platform(
            &semver::Version::new(0, 13, 0),
            turborepo_setup::lock::Platform::WindowsX64,
        )
        .unwrap();
        let lock = json!({"schemaVersion":0,"tools":{
            "python":{"adapter":"python","version":"3.12.12",
                "declarations":[{"file":".python-version","request":"3.12"}],
                "installation":{"kind":"managed","artifacts":{"windows-x64":{"distribution":{
                    "url":format!("https://releases.astral.sh{BUILD}"),
                    "sha256":format!("{:x}", Sha256::digest(&python)),"format":"tar-gz",
                    "rootPrefix":"python","destination":KEY,
                    "executables":{"python":"python.exe"}}}}}},
            "uv":{"adapter":"uv","version":"0.13.0",
                "declarations":[{"file":"pyproject.toml","field":"tool.uv.required-version",
                    "request":">=0.13"}],
                "installation":{"kind":"managed","artifacts":{"windows-x64":{
                    "distribution":official.lock_artifact(format!("{:x}", Sha256::digest(&uv)))}}}}
        }});
        fs::write(root.join("turbo.lock"), lock.to_string()).unwrap();
        let path = official.url()["https://releases.astral.sh".len()..].to_owned();
        let upstream = LoopbackServer::new([(path, uv), (BUILD.into(), python)], |_| {}).unwrap();
        Self { owned, upstream }
    }
    fn run(&self, flags: &[&str]) -> Result<i32, Error> {
        let cwd = self.owned.root().join("apps/web/src");
        let mut words = vec!["turbo".into(), "setup".into(), "--cwd".into()];
        words.push(cwd.clone().into_os_string());
        words.extend(flags.iter().map(std::ffi::OsString::from));
        let args = Args::parse_args(words).unwrap();
        let origin = self.upstream.origin();
        crate::cli::dispatch_setup(&args, |args, setup_args| {
            run_with_policy(
                args,
                setup_args,
                Some(provision::Transports {
                    node: turborepo_setup::node_provision::NodeTransport::loopback_http_for_tests(origin).unwrap(),
                    pnpm: turborepo_setup::pnpm_provision::PnpmTransport::loopback_http_for_tests(origin).unwrap(),
                    registry: turborepo_setup::registry_resolution::RegistryTransport::loopback_http_for_tests(origin).unwrap(),
                    tool_store: Some(self.owned.home().to_owned()),
                    rustup: turborepo_setup::rustup_provision::RustupTransport::loopback_http_for_tests(origin).unwrap(),
                    uv: turborepo_setup::uv_resolution::UvTransport::loopback_http_for_tests(origin).unwrap(),
                    python: turborepo_setup::python_resolution::PythonTransport::loopback_http_for_tests(origin).unwrap(),
                    go: turborepo_setup::go_resolution::GoTransport::loopback_http_for_tests(origin).unwrap(),
                    bun: turborepo_setup::bun_provision::BunTransport::loopback_http_for_tests(origin).unwrap(),
                    artifact: None,
                }),
                turborepo_setup::git::FakeGit::shared(),
                || Ok(self.owned.policy_at(&cwd)?),
            )
        })
        .unwrap()
    }
    /// What `turbo run` and `turbo exec` apply: PATH plus runtime variables.
    fn activation(&self) -> (turborepo_run::EnvironmentVariableMap, PathBuf) {
        use crate::commands::managed::{ManagedTools, context};
        let root = self.owned.root();
        let config = root.join("turbo.json");
        let tools = ManagedTools::inspect(root, &config, true, Some(self.owned.home()), context)
            .unwrap()
            .unwrap();
        let env = tools.revalidate().unwrap();
        let bin = std::env::split_paths(&env["PATH"]).next().unwrap();
        (env, bin)
    }
    /// The recorded environment's record, tools and tree roots.
    fn installed(&self) -> RecordedInstallation {
        recorded_installation_for_tests(self.owned.root(), self.owned.home())
            .unwrap()
            .unwrap()
    }
}

#[test]
fn frozen_setup_exports_uv_and_python_and_activation_pins_the_managed_interpreter() {
    if unqualified_host() {
        return;
    }
    let repo = Repo::new("cmd.exe");
    assert_eq!(repo.run(FROZEN).unwrap(), 0);
    assert_eq!(repo.upstream.hits(), 2);
    let (env, bin) = repo.activation();
    let mut names: Vec<_> = fs::read_dir(&bin)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["python.cmd", "uv.exe", "uvx.exe"]);
    // uv runs as itself; python runs in place through its launcher.
    let path = std::env::join_paths([bin.clone(), system32()]).unwrap();
    assert_eq!(exit_code(&path, "uv /d /c exit 5"), Some(5));
    assert_eq!(exit_code(&path, "uvx /d /c exit 6"), Some(6));
    assert_eq!(exit_code(&path, "python /d /c exit 4"), Some(4));
    // Every uv process gets the launchers' settings, with ordinary paths.
    let installed = repo.installed();
    let python_dir = &installed.trees["python"];
    assert_eq!(env["UV_PYTHON_INSTALL_DIR"], python_dir.to_str().unwrap());
    assert!(!env["UV_PYTHON_INSTALL_DIR"].starts_with(r"\\?\"));
    assert!(python_dir.join(KEY).join("python.exe").is_file());
    assert!(python_dir.join(KEY).join("Lib").join("os.py").is_file());
    assert_eq!(env["UV_PYTHON"], KEY);
    assert_eq!(env["UV_PYTHON_DOWNLOADS"], "never");
    assert_eq!(env["UV_PYTHON_PREFERENCE"], "only-managed");
    let record = installed.record;
    for flags in [FROZEN, &["--check", "--tools-only", "--offline"][..]] {
        assert_eq!(repo.run(flags).unwrap(), 0);
    }
    assert_eq!(repo.upstream.hits(), 2);
    assert_eq!(repo.installed().record, record);
}

#[test]
fn plain_setup_syncs_locked_uv_workspaces_through_the_managed_uv_exe() {
    if unqualified_host() {
        return;
    }
    const UV_LOCK: &[u8] = b"version = 1\n";
    // cmd.exe as uv.exe ends at stdin's EOF and succeeds.
    let repo = Repo::new("cmd.exe");
    fs::write(repo.owned.root().join("uv.lock"), UV_LOCK).unwrap();
    assert_eq!(repo.run(&["--frozen"]).unwrap(), 0);
    assert_eq!(
        fs::read(repo.owned.root().join("uv.lock")).unwrap(),
        UV_LOCK
    );
    // whoami.exe as uv.exe rejects `sync` as an invalid option; setup reports
    // uv's failure status instead of success.
    let repo = Repo::new("whoami.exe");
    fs::write(repo.owned.root().join("uv.lock"), UV_LOCK).unwrap();
    let code = repo.run(&["--frozen"]).unwrap();
    assert_ne!(code, 0);
    assert_eq!(repo.run(&["--frozen", "--tools-only"]).unwrap(), 0);
    assert_eq!(
        fs::read(repo.owned.root().join("uv.lock")).unwrap(),
        UV_LOCK
    );
}
