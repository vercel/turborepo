//! Milestone 2 exit journey through the real binary: global
//! `turbo setup --tools-only` in a Node/pnpm repository, then plain
//! `turbo run`. The host PATH only holds refusing node/npm/pnpm/corepack
//! sentinels plus Git. The mock upstream is a TLS-terminating CONNECT proxy
//! reached through the binary's standard `HTTPS_PROXY`/`SSL_CERT_FILE`
//! support. It serves fixture bytes for the two official origins and refuses
//! anything else, so every network attempt is observable.
#![cfg(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_env = "gnu", target_arch = "x86_64")
))]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use serde_json::json;
use sha2::{Digest, Sha256, Sha512};
use turborepo_setup::test_support::LoopbackServer;
use turborepo_shim::TurboState;

const CA: &[u8] = include_bytes!("fixtures/setup_upstream/ca.pem");
const CERT: &[u8] = include_bytes!("fixtures/setup_upstream/server.pem");
const KEY: &[u8] = include_bytes!("fixtures/setup_upstream/server-key.pem");
const ORIGINS: [&str; 2] = ["nodejs.org:443", "registry.npmjs.org:443"];
const TOOLS_ONLY: &[&str] = &["setup", "--tools-only"];
// Managed pnpm reports how turbo dispatched it, then runs `node` from PATH.
const PNPM: &str =
    "#!/bin/sh\necho \"fixture-pnpm 10.0.0 secret=${JOURNEY_SECRET:-filtered}\"\nexec node\n";

fn executable(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn tar_gz(prefix: &str, files: &[(&str, &str)], links: &[(&str, &str)]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, body) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, format!("{prefix}/{path}"), body.as_bytes())
            .unwrap();
    }
    for (path, target) in links {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_link_name(target).unwrap();
        header.set_cksum();
        tar.append_data(&mut header, format!("{prefix}/{path}"), &b""[..])
            .unwrap();
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}

fn node(prefix: &str, version: &str) -> Vec<u8> {
    let cli = "#!/bin/sh\nexit 97\n";
    // Setup's pnpm launcher runs `node pnpm.cjs`; emulate a JS entrypoint.
    let node = format!(
        "#!/bin/sh\ncase \"$1\" in *.cjs) entry=$1; shift; exec /bin/sh \"$entry\" \"$@\";; \
         esac\necho \"fixture-node {version}\"\n"
    );
    tar_gz(
        prefix,
        &[
            ("bin/node", &node),
            (
                "lib/node_modules/npm/package.json",
                r#"{"name":"npm","version":"11.6.1","bin":{"npm":"bin/npm-cli.js","npx":"bin/npx-cli.js"}}"#,
            ),
            ("lib/node_modules/npm/bin/npm-cli.js", cli),
            ("lib/node_modules/npm/bin/npx-cli.js", cli),
        ],
        &[
            ("bin/npm", "../lib/node_modules/npm/bin/npm-cli.js"),
            ("bin/npx", "../lib/node_modules/npm/bin/npx-cli.js"),
        ],
    )
}

/// Official-origin routes for Node 24.0.0/24.1.0 and pnpm 10.0.0. Only this
/// host's Node archive is served; the lock still records every platform.
fn routes() -> Vec<(String, Vec<u8>)> {
    let host = if cfg!(target_os = "macos") {
        "darwin-arm64"
    } else {
        "linux-x64"
    };
    let versions = ["24.1.0", "24.0.0"];
    let mut routes = Vec::new();
    for version in versions {
        let mut sums = String::new();
        for target in [
            "darwin-x64",
            "darwin-arm64",
            "linux-x64",
            "linux-arm64",
            "win-x64",
            "win-arm64",
        ] {
            let prefix = format!("node-v{version}-{target}");
            let extension = if target.starts_with("win-") {
                "zip"
            } else {
                "tar.gz"
            };
            let file = format!("{prefix}.{extension}");
            let bytes = if target == host {
                node(&prefix, version)
            } else {
                file.clone().into_bytes()
            };
            sums.push_str(&format!("{:x}  {file}\n", Sha256::digest(&bytes)));
            if target == host {
                routes.push((format!("/dist/v{version}/{file}"), bytes));
            }
        }
        routes.push((
            format!("/dist/v{version}/SHASUMS256.txt"),
            sums.into_bytes(),
        ));
    }
    let index = versions.map(|version| {
        json!({"version":format!("v{version}"),"npm":"11.6.1","lts":"Krypton",
            "files":["osx-x64-tar","osx-arm64-tar","linux-x64","linux-arm64","win-x64-zip","win-arm64-zip"]})
    });
    routes.push((
        "/dist/index.json".into(),
        serde_json::to_vec(&index).unwrap(),
    ));
    let pnpm = tar_gz(
        "package",
        &[
            (
                "package.json",
                r#"{"name":"pnpm","version":"10.0.0","bin":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}}"#,
            ),
            ("bin/pnpm.cjs", PNPM),
            ("bin/pnpx.cjs", "#!/bin/sh\nexit 97\n"),
        ],
        &[],
    );
    let metadata = json!({"name":"pnpm","version":"10.0.0","dist":{
        "tarball":"https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz",
        "integrity":format!("sha512-{}", STANDARD.encode(Sha512::digest(&pnpm)))}});
    routes.push(("/pnpm/10.0.0".into(), metadata.to_string().into_bytes()));
    routes.push(("/pnpm/-/pnpm-10.0.0.tgz".into(), pnpm));
    // The same bytes beneath the path-prefixed mirror bases the mirror
    // journey configures.
    let mirrored: Vec<_> = routes
        .iter()
        .flat_map(|(path, body)| {
            let (bases, rest): (&[&str], _) = match path.strip_prefix("/dist") {
                Some(rest) => (&["/policy/node", "/env/node"], rest),
                None => (&["/policy/npm"], path.as_str()),
            };
            bases
                .iter()
                .map(move |base| (format!("{base}{rest}"), body.clone()))
        })
        .collect();
    routes.extend(mirrored);
    routes
}

/// TLS-terminating CONNECT proxy in front of the owned loopback fixture. It
/// records every CONNECT target and never resolves or contacts the request's
/// authority.
struct Upstream {
    origin: LoopbackServer,
    proxy: String,
    connects: Arc<Mutex<Vec<String>>>,
}

impl Upstream {
    fn new() -> Self {
        let origin = LoopbackServer::new(routes(), |_| {}).unwrap();
        let address = origin.origin().trim_start_matches("http://").to_owned();
        let tls = Arc::new(
            rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from_pem_slice(CERT).unwrap()],
                PrivateKeyDer::from_pem_slice(KEY).unwrap(),
            )
            .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = format!("http://{}", listener.local_addr().unwrap());
        let connects = Arc::new(Mutex::new(Vec::new()));
        let log = connects.clone();
        // Detached: the accept loop ends with the test process.
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let (tls, log, address) = (tls.clone(), log.clone(), address.clone());
                thread::spawn(move || {
                    let _ = tunnel(client, tls, &log, &address);
                });
            }
        });
        Self {
            origin,
            proxy,
            connects,
        }
    }

    fn connects(&self) -> Vec<String> {
        self.connects.lock().unwrap().clone()
    }
}

fn tunnel(
    mut client: TcpStream,
    tls: Arc<rustls::ServerConfig>,
    log: &Mutex<Vec<String>>,
    origin: &str,
) -> io::Result<()> {
    let head = String::from_utf8_lossy(&read_head(&mut client)?).into_owned();
    let target = head
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    // Logged before replying, so it is visible once the CLI sees a response.
    log.lock().unwrap().push(target.clone());
    if !head.starts_with("CONNECT ") || !ORIGINS.contains(&target.as_str()) {
        return client.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
    }
    client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    let session = rustls::ServerConnection::new(tls).map_err(io::Error::other)?;
    let mut stream = rustls::StreamOwned::new(session, client);
    let request = read_head(&mut stream)?;
    let mut upstream = TcpStream::connect(origin)?;
    upstream.write_all(&request)?;
    let mut response = Vec::new();
    upstream.read_to_end(&mut response)?;
    stream.write_all(&response)?;
    stream.conn.send_close_notify();
    stream.flush()
}

/// Bounded HTTP head read, byte by byte so nothing past the head is consumed.
fn read_head(stream: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 16 * 1024 || stream.read(&mut byte)? == 0 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        head.push(byte[0]);
    }
    Ok(head)
}

fn contains(text: &str, expected: &str) -> bool {
    // Diagnostics may wrap sentences across lines with gutters.
    let squash = |s: &str| s.to_lowercase().replace([' ', '\n', '|', '│'], "");
    squash(text).contains(&squash(expected))
}

struct Outcome {
    code: i32,
    text: String,
    connects: Vec<String>,
}

impl Outcome {
    fn succeeds(&self) -> &Self {
        assert_eq!(self.code, 0, "{}", self.text);
        self
    }

    /// Fails before any task runs, with the expected guidance.
    fn fails(&self, expected: &str) -> &Self {
        assert_ne!(self.code, 0, "{}", self.text);
        assert!(
            contains(&self.text, expected),
            "{expected:?}: {}",
            self.text
        );
        assert!(!self.text.contains("fixture-"), "{}", self.text);
        self
    }

    fn offline(&self) -> &Self {
        assert!(
            self.connects.is_empty(),
            "{:?}: {}",
            self.connects,
            self.text
        );
        self
    }
}

/// Bounds every `turbo watch` observation: a regression fails, never hangs.
const WATCH_TIMEOUT: Duration = Duration::from_secs(60);

/// A running `turbo watch`, killed on drop.
struct Watch {
    child: Child,
    log: PathBuf,
}

impl Watch {
    fn output(&self) -> String {
        fs::read_to_string(&self.log).unwrap()
    }

    /// Waits until the output has every expected line while watch runs on.
    fn running_until(&mut self, expected: &[&str]) {
        let deadline = Instant::now() + WATCH_TIMEOUT;
        while !expected.iter().all(|line| self.output().contains(line)) {
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "{}",
                self.output()
            );
            assert!(Instant::now() < deadline, "no task ran: {}", self.output());
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// The exit code, if watch stops on its own within `timeout`.
    fn exits_within(&mut self, timeout: Duration) -> Option<i32> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().unwrap() {
                return Some(status.code().unwrap_or(-1));
            }
            thread::sleep(Duration::from_millis(100));
        }
        None
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Journey {
    temp: tempfile::TempDir,
    root: PathBuf,
    web: PathBuf,
    home: PathBuf,
    path: OsString,
    sentinel: PathBuf,
    ca: PathBuf,
    upstream: Upstream,
}

impl Journey {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        let web = root.join("apps/web");
        let home = temp.path().join("home");
        let poisoned = temp.path().join("host-bin");
        let host = temp.path().join("host-tools");
        let sentinel = temp.path().join("host-tool-ran");
        for dir in [&web, &home, &poisoned, &host] {
            fs::create_dir_all(dir).unwrap();
        }
        for tool in ["node", "npm", "npx", "pnpm", "pnpx", "corepack", "yarn"] {
            let script = format!(
                "#!/bin/sh\necho {tool} >> '{}'\nexit 97\n",
                sentinel.display()
            );
            executable(&poisoned.join(tool), &script);
        }
        std::os::unix::fs::symlink(which::which("git").unwrap(), host.join("git")).unwrap();
        let ca = temp.path().join("ca.pem");
        fs::write(&ca, CA).unwrap();
        for (name, contents) in [
            (
                "turbo.json",
                r#"{"futureFlags":{"experimentalSetup":true},"agentGuidance":false,"tasks":{"build":{"cache":false}}}"#,
            ),
            (".nvmrc", "24.0.0\n"),
            (".gitignore", "/.turbo/\nnode_modules/\n"),
            (
                "package.json",
                r#"{"name":"journey","private":true,"packageManager":"pnpm@10.0.0"}"#,
            ),
            ("pnpm-workspace.yaml", "packages: ['apps/*']\n"),
            (
                "pnpm-lock.yaml",
                "lockfileVersion: '9.0'\nimporters:\n  .: {}\n  apps/web: {}\n",
            ),
            (
                "apps/web/package.json",
                r#"{"name":"web","scripts":{"build":"node"}}"#,
            ),
        ] {
            fs::write(root.join(name), contents).unwrap();
        }
        common::git(&root, &["init", "--quiet"]);
        Self {
            path: std::env::join_paths([poisoned, host]).unwrap(),
            upstream: Upstream::new(),
            temp,
            root,
            web,
            home,
            sentinel,
            ca,
        }
    }

    fn turbo(&self, cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Outcome {
        let seen = self.upstream.connects().len();
        let output = self.command(cwd, args, env).output().unwrap();
        assert!(
            !self.sentinel.exists(),
            "host tool ran: {}",
            fs::read_to_string(&self.sentinel).unwrap_or_default()
        );
        Outcome {
            code: output.status.code().unwrap(),
            text: common::combined_output(&output),
            connects: self.upstream.connects()[seen..].to_vec(),
        }
    }

    /// `turbo watch` from the root, its combined output in a file.
    fn watch(&self, args: &[&str]) -> Watch {
        let (file, log) = tempfile::NamedTempFile::new_in(self.temp.path())
            .unwrap()
            .keep()
            .unwrap();
        let child = self
            .command(&self.root, args, &[])
            .stdout(file.try_clone().unwrap())
            .stderr(file)
            .spawn()
            .unwrap();
        Watch { child, log }
    }

    fn command(&self, cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_turbo"));
        command
            .env_clear()
            .current_dir(cwd)
            .args(args)
            .env("PATH", &self.path)
            .env("HOME", &self.home)
            .env("TURBO_CONFIG_DIR_PATH", self.home.join("turbo-config"))
            .env("TURBO_TELEMETRY_DISABLED", "1")
            .env("DO_NOT_TRACK", "1")
            .env("TURBO_NO_UPDATE_NOTIFIER", "1")
            .env("HTTPS_PROXY", &self.upstream.proxy)
            .env("HTTP_PROXY", &self.upstream.proxy)
            .env("ALL_PROXY", &self.upstream.proxy)
            .env("NO_PROXY", "")
            .env("SSL_CERT_FILE", &self.ca)
            .env("JOURNEY_SECRET", "present")
            .envs(env.iter().copied());
        command
    }

    /// The same repository with a root `setup.mirrors` policy.
    fn mirrored(mirrors: &str) -> Self {
        let j = Self::new();
        let config = format!(
            r#"{{"futureFlags":{{"experimentalSetup":true}},"agentGuidance":false,"setup":{{"mirrors":{mirrors}}},"tasks":{{"build":{{"cache":false}}}}}}"#
        );
        fs::write(j.root.join("turbo.json"), config).unwrap();
        j
    }

    fn read(&self, path: &str) -> Vec<u8> {
        fs::read(self.root.join(path)).unwrap()
    }

    fn store(&self) -> PathBuf {
        common::setup::user_tool_store(&self.home, None)
    }

    /// The repository's setup record.
    fn record(&self) -> Vec<u8> {
        common::setup::installed(&self.root, &self.store())
            .expect("recorded installation")
            .record
    }

    /// Plain `turbo run build`: managed pnpm dispatch with managed Node first
    /// on the task PATH, env-mode filtering, no network and no setup writes.
    fn build(&self, cwd: &Path, mode: &str, node: &str, extra: &[&str]) -> Outcome {
        let lock = self.read("turbo.lock");
        let manifest = self.record();
        let env_mode = format!("--env-mode={mode}");
        let mut args = vec!["run", "build", env_mode.as_str()];
        args.extend(extra);
        let outcome = self.turbo(cwd, &args, &[]);
        outcome.succeeds().offline();
        let secret = if mode == "loose" {
            "present"
        } else {
            "filtered"
        };
        for expected in [
            format!("fixture-pnpm 10.0.0 secret={secret}"),
            format!("fixture-node {node}"),
        ] {
            assert!(outcome.text.contains(&expected), "{}", outcome.text);
        }
        assert_eq!(self.read("turbo.lock"), lock);
        assert_eq!(self.record(), manifest);
        outcome
    }
}

#[test]
fn global_setup_then_plain_run_repeat_repair_and_drift() {
    let j = Journey::new();
    let ci = [("CI", "1")];
    j.turbo(&j.root, TOOLS_ONLY, &ci)
        .fails("frozen mode requires turbo.lock")
        .offline();
    assert!(!j.root.join("turbo.lock").exists());

    // First setup from a subdirectory creates the lock and asks for a commit.
    let first = j.turbo(&j.web, TOOLS_ONLY, &[]);
    first.succeeds();
    assert!(
        contains(&first.text, "turbo.lock created, commit it"),
        "{}",
        first.text
    );
    assert!(!first.connects.is_empty() && !first.text.contains("fixture-"));
    let lock = j.read("turbo.lock");
    for cwd in [&j.root, &j.web] {
        j.build(cwd, "strict", "24.0.0", &[]);
        j.build(cwd, "loose", "24.0.0", &[]);
    }

    // Repeat setup is a no-op without network.
    let manifest = j.record();
    j.turbo(&j.root, TOOLS_ONLY, &[]).succeeds().offline();
    assert_eq!(j.read("turbo.lock"), lock);
    assert_eq!(j.record(), manifest);

    // A deleted shim fails before tasks without network; setup repairs it.
    let current = common::setup::installed(&j.root, &j.store()).unwrap();
    fs::remove_file(current.bin.join("pnpm")).unwrap();
    for env in [&[][..], &ci[..]] {
        j.turbo(&j.web, &["run", "build"], env)
            .fails("run turbo setup")
            .offline();
    }
    // JavaScript discovery runs no tool, so it needs no managed selection.
    j.turbo(&j.web, &["ls"], &[]).succeeds().offline();
    j.turbo(&j.root, TOOLS_ONLY, &[]).succeeds();
    assert_eq!(j.read("turbo.lock"), lock);
    j.build(&j.web, "strict", "24.0.0", &[]);

    // Declaration drift: run points at setup, frozen setup rejects the stale
    // lock, and local setup updates it before the next run.
    fs::write(j.root.join(".nvmrc"), "24.1.0\n").unwrap();
    for env in [&[][..], &ci[..]] {
        j.turbo(&j.web, &["run", "build"], env)
            .fails("turbo setup")
            .offline();
        j.turbo(&j.root, TOOLS_ONLY, &ci)
            .fails("node: .nvmrc")
            .fails("turbo setup --tools-only")
            .fails("locally and commit the updated turbo.lock")
            .offline();
    }
    assert_eq!(j.read("turbo.lock"), lock);
    let update = j.turbo(&j.root, TOOLS_ONLY, &[]);
    update.succeeds();
    assert!(
        contains(&update.text, "node 24.0.0 → 24.1.0"),
        "{}",
        update.text
    );
    j.build(&j.root, "strict", "24.1.0", &[]);
    j.build(&j.web, "loose", "24.1.0", &[]);
}

/// Setup records the repository in the user's tool store, never in the
/// repository, and `TURBO_TOOL_STORE` relocates that store.
#[test]
fn setup_records_the_repository_in_the_user_tool_store() {
    let j = Journey::new();
    j.turbo(&j.root, TOOLS_ONLY, &[]).succeeds();
    let root = j.root.canonicalize().unwrap();
    let name = format!("{:x}", Sha256::digest(root.as_os_str().as_encoded_bytes()));
    let data = if cfg!(target_os = "macos") {
        j.home.join("Library/Application Support")
    } else {
        j.home.join(".local/share")
    };
    let record = |store: &Path| store.join("repos").join(&name).join("record.json");
    let default = data.join("turborepo/tools");
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(record(&default)).unwrap()).unwrap();
    assert_eq!(value["root"], json!(root));
    assert!(value["env"].is_string());
    assert!(!data.join("turborepo/setup-roots").exists());
    assert!(!j.root.join(".turbo/cargo-home").exists());
    j.build(&j.root, "strict", "24.0.0", &[]);

    // A relocated store has no record until setup writes one there.
    let store = j.temp.path().join("tool-store");
    let env = [("TURBO_TOOL_STORE", store.to_str().unwrap())];
    let build = ["run", "build"];
    j.turbo(&j.root, &build, &env)
        .fails("run turbo setup")
        .offline();
    j.turbo(&j.root, TOOLS_ONLY, &env).succeeds();
    assert!(record(&store).is_file());
    let run = j.turbo(&j.root, &build, &env);
    run.succeeds().offline();
    assert!(run.text.contains("fixture-node 24.0.0"), "{}", run.text);

    // A relative store or one inside the repository is refused before any
    // traffic, and nothing is created there.
    for (path, created) in [
        ("relative-store", j.root.join("relative-store")),
        (".turbo/store", j.root.join(".turbo/store")),
    ] {
        let path = if path.starts_with('.') {
            j.root.join(path).to_str().unwrap().to_owned()
        } else {
            path.to_owned()
        };
        for args in [TOOLS_ONLY, &build[..]] {
            j.turbo(&j.root, args, &[("TURBO_TOOL_STORE", &path)])
                .fails("TURBO_TOOL_STORE must name an absolute directory")
                .offline();
        }
        assert!(!created.exists(), "{}", created.display());
    }
}

/// Tool acquisition ignores npm's ambient configuration: first setup still
/// uses verified official fixture downloads, and installed repeats stay
/// offline. All environment overrides belong to child commands, never the test
/// process.
#[test]
fn ambient_npm_configuration_does_not_affect_tool_downloads_or_installed_repeats() {
    let clean = Journey::new();
    clean.turbo(&clean.root, TOOLS_ONLY, &[]).succeeds();
    let expected_lock = clean.read("turbo.lock");
    let mut expected_requests = clean.upstream.origin.requests();
    expected_requests.sort();
    assert!(!expected_requests.is_empty());

    let j = Journey::new();
    let npmrc = "registry=https://registry.example.invalid/\n\
                 proxy=http://127.0.0.1:1\nhttps-proxy=http://127.0.0.1:1\n\
                 cafile=/nonexistent/npm-ca.pem\nstrict-ssl=false\noffline=true\n";
    let global = j.temp.path().join("global.npmrc");
    let user = j.home.join(".npmrc");
    let prefix = j.temp.path().join("npm-prefix");
    fs::create_dir_all(prefix.join("etc")).unwrap();
    fs::create_dir_all(prefix.join("lib/node_modules/npm")).unwrap();
    for path in [
        j.temp.path().join(".npmrc"),
        j.root.join(".npmrc"),
        j.web.join(".npmrc"),
        user.clone(),
        global.clone(),
        prefix.join("etc/npmrc"),
        prefix.join("lib/node_modules/npm/npmrc"),
    ] {
        fs::write(path, npmrc).unwrap();
    }
    let ambient = [
        ("npm_config_registry", "https://lowercase.example.invalid/"),
        ("NPM_CONFIG_REGISTRY", "https://uppercase.example.invalid/"),
        ("npm_config_userconfig", user.to_str().unwrap()),
        ("NPM_CONFIG_GLOBALCONFIG", global.to_str().unwrap()),
        ("npm_config_prefix", prefix.to_str().unwrap()),
        ("npm_config_global_prefix", prefix.to_str().unwrap()),
        ("npm_config_proxy", "http://127.0.0.1:1"),
        ("npm_config_https_proxy", "http://127.0.0.1:1"),
        ("npm_config_cafile", "/nonexistent/npm-ca.pem"),
        ("npm_config_strict_ssl", "false"),
        ("npm_config_offline", "true"),
        ("npm_config_unrecognized_setting", "ignored"),
        ("COREPACK_NPM_REGISTRY", "https://corepack.example.invalid/"),
        ("DESTDIR", "/nonexistent/ambient-prefix"),
        ("UV_INDEX_URL", "https://python.example.invalid/simple"),
        ("PIP_INDEX_URL", "https://pip.example.invalid/simple"),
        ("GOPROXY", "https://go.example.invalid"),
        ("RUSTUP_DIST_SERVER", "https://rust.example.invalid"),
        ("NODE_MIRROR", "https://node-mirror.example.invalid/"),
        (
            "NODEJS_ORG_MIRROR",
            "https://nodejs-mirror.example.invalid/",
        ),
        (
            "NVM_NODEJS_ORG_MIRROR",
            "https://nvm-mirror.example.invalid/",
        ),
    ];
    let first = j.turbo(&j.web, TOOLS_ONLY, &ambient);
    first.succeeds();
    assert!(!first.connects.is_empty(), "{}", first.text);
    assert!(!first.text.contains("fixture-"), "{}", first.text);
    assert_eq!(j.read("turbo.lock"), expected_lock);
    let mut requests = j.upstream.origin.requests();
    requests.sort();
    assert_eq!(requests, expected_requests);
    // The recorded installation, not just exit status.
    let installed =
        common::setup::installed(&j.root, &j.store()).expect("recorded managed installation");
    assert!(installed.bin.join("node").exists());
    assert!(installed.bin.join("pnpm").exists());
    let manifest = j.record();

    // Unresolved npm interpolation is also irrelevant to tools-only reuse.
    fs::write(
        j.root.join(".npmrc"),
        "not an npm setting = ${UNSET_NPM_VALUE}\n",
    )
    .unwrap();
    let mut ci = ambient.to_vec();
    ci.push(("CI", "1"));
    for args in [
        TOOLS_ONLY,
        &["setup", "--tools-only", "--offline"][..],
        &["setup", "--tools-only", "--check", "--offline"][..],
    ] {
        j.turbo(&j.root, args, &ci).succeeds().offline();
        assert_eq!(j.read("turbo.lock"), expected_lock);
        assert_eq!(j.record(), manifest);
        assert!(common::setup::installed(&j.root, &j.store()).is_some());
    }
    assert_eq!(j.upstream.origin.requests().len(), requests.len());
}

#[test]
fn repository_local_cli_pins_after_global_setup() {
    let j = Journey::new();
    let first = j.turbo(&j.root, TOOLS_ONLY, &[]);
    first.succeeds();
    // No pin: the global CLI runs the guarded managed pipeline itself.
    j.build(&j.web, "strict", "24.0.0", &[]);

    // An incompatible pin appearing after setup stops before tasks with
    // upgrade guidance; nothing is substituted or fetched.
    let pin = j
        .root
        .join("node_modules")
        .join(TurboState::scoped_platform_package_scope())
        .join(TurboState::scoped_platform_package_dir());
    fs::create_dir_all(pin.join("bin")).unwrap();
    let binary = pin.join("bin").join(TurboState::binary_name());
    let calls = j.temp.path().join("pin-calls");
    let old = format!(
        "#!/bin/sh\necho \"$@\" >> '{}'\necho \"error: unexpected argument '$1' found\" >&2\nexit \
         2\n",
        calls.display()
    );
    executable(&binary, &old);
    fs::write(pin.join("package.json"), r#"{"version":"2.5.0"}"#).unwrap();
    for cwd in [&j.root, &j.web] {
        let outcome = j.turbo(cwd, &["run", "build"], &[]);
        outcome
            .fails("repository-local turbo 2.5.0")
            .fails("does not report managed-run support")
            .fails("upgrade")
            .offline();
    }
    let queried = fs::read_to_string(&calls).unwrap();
    assert!(
        queried
            .lines()
            .all(|line| line == "--__internal-managed-run-capabilities"),
        "{queried}"
    );
    // Setup stays global: it neither defers to nor runs the pin.
    j.turbo(&j.root, TOOLS_ONLY, &[]).succeeds().offline();
    assert_eq!(fs::read_to_string(&calls).unwrap(), queried);

    // A compatible copied pin is preserved, and the managed contract holds
    // through the handoff.
    fs::remove_file(&binary).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_turbo"), &binary).unwrap();
    let version = include_str!("../../../version.txt").lines().next().unwrap();
    fs::write(
        pin.join("package.json"),
        json!({"version": version}).to_string(),
    )
    .unwrap();
    let handoff = format!(
        "Running local turbo binary in {}",
        fs::canonicalize(&binary).unwrap().display()
    );
    for cwd in [&j.root, &j.web] {
        for mode in ["strict", "loose"] {
            let outcome = j.build(cwd, mode, "24.0.0", &["--verbosity=2"]);
            assert!(outcome.text.contains(&handoff), "{}", outcome.text);
        }
    }
    fs::write(j.root.join(".nvmrc"), "24.1.0\n").unwrap();
    j.turbo(&j.web, &["run", "build"], &[])
        .fails("turbo setup")
        .offline();
}

/// `turbo watch` activates managed tools exactly like `turbo run`: the initial
/// run dispatches managed pnpm and Node, and declaration drift stops a rerun
/// or startup with `turbo setup` guidance instead of idling silently.
#[test]
fn watch_uses_managed_tools_and_fails_on_drift() {
    let j = Journey::new();
    let first = j.turbo(&j.root, TOOLS_ONLY, &[]);
    first.succeeds();
    let commit = |message: &str| {
        common::git(&j.root, &["add", "."]);
        common::git(&j.root, &["commit", "--quiet", "-m", message]);
    };
    common::git(&j.root, &["config", "user.email", "journey@example.com"]);
    common::git(&j.root, &["config", "user.name", "journey"]);
    commit("setup");
    let seen = j.upstream.connects().len();
    let args = ["watch", "build", "--ui=stream"];

    let mut watch = j.watch(&args);
    watch.running_until(&["fixture-pnpm 10.0.0 secret=filtered", "fixture-node 24.0.0"]);
    fs::write(j.root.join(".nvmrc"), "24.1.0\n").unwrap();
    // Any package change reruns; retry in case the watcher coalesces events.
    let code = (0..3).find_map(|attempt| {
        fs::write(j.web.join("index.js"), format!("{attempt}\n")).unwrap();
        commit("change");
        watch.exits_within(Duration::from_secs(20))
    });
    let text = watch.output();
    assert!(code.is_some_and(|code| code != 0), "{text}");
    assert!(contains(&text, "run turbo setup"), "{text}");
    assert_eq!(text.matches("fixture-node").count(), 1, "{text}");

    // With the drift in place, watch fails at startup before any task.
    let mut watch = j.watch(&args);
    let code = watch.exits_within(WATCH_TIMEOUT);
    Outcome {
        code: code.unwrap_or_else(|| panic!("watch idled: {}", watch.output())),
        text: watch.output(),
        connects: j.upstream.connects()[seen..].to_vec(),
    }
    .fails("run turbo setup")
    .offline();
    assert!(!j.sentinel.exists(), "a host tool ran");
}

/// TURBO-6404: root `setup.mirrors` routes downloads, an environment variable
/// overrides one mirror, and `turbo.lock` keeps the official URLs either way.
/// Each policy mirror sits on the other official host, so the CONNECT log
/// shows which base served each tool.
#[test]
fn policy_mirrors_route_downloads_and_environment_overrides_them() {
    let official = Journey::new();
    let first = official.turbo(&official.root, TOOLS_ONLY, &[]);
    first.succeeds();
    let lock = official.read("turbo.lock");
    let policy = r#"{"node":"https://registry.npmjs.org/policy/node/","npmRegistry":"https://nodejs.org/policy/npm"}"#;
    let env = [("TURBO_SETUP_NODE_MIRROR", "https://nodejs.org/env/node")];
    for (env, node, hosts) in [
        (&[][..], "/policy/node/", &ORIGINS[..]),
        (&env[..], "/env/node/", &ORIGINS[..1]),
    ] {
        let j = Journey::mirrored(policy);
        let outcome = j.turbo(&j.web, TOOLS_ONLY, env);
        outcome.succeeds();
        assert_eq!(j.read("turbo.lock"), lock, "{}", outcome.text);
        let requests = j.upstream.origin.requests();
        for base in [node, "/policy/npm/"] {
            assert!(requests.iter().any(|path| path.starts_with(base)));
        }
        assert!(
            requests
                .iter()
                .all(|path| path.starts_with(node) || path.starts_with("/policy/npm/")),
            "{requests:?}"
        );
        let mut connects = outcome.connects.clone();
        connects.sort();
        connects.dedup();
        let mut expected = hosts.to_vec();
        expected.sort();
        assert_eq!(connects, expected);
    }
}

/// Invalid committed mirrors fail before any traffic or write, naming the key
/// and masking the value, for setup and for every other command.
#[test]
fn invalid_policy_mirrors_fail_before_traffic_without_echo() {
    let j = Journey::mirrored(r#"{"node":"https://user:s3cret@mirror.example/dist"}"#);
    for args in [TOOLS_ONLY, &["run", "build"][..]] {
        let outcome = j.turbo(&j.root, args, &[]);
        outcome
            .fails("setup.mirrors.node must be an absolute HTTPS URL")
            .offline();
        assert!(!outcome.text.contains("s3cret"), "{}", outcome.text);
    }
    assert!(!j.root.join("turbo.lock").exists() && !j.root.join(".turbo").exists());
}
