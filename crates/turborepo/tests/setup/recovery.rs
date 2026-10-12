//! Real `turbo setup` processes killed mid-download and mid-unpack, then
//! repeated, corrupted, and offline. Production has no loopback override, so
//! the shared download cache is the only artifact source and a proxy that
//! never answers stands in for a stalled network.
#![allow(clippy::unwrap_used)] // Fixture failures fail the test.

use std::{
    fs,
    io::{self, Write},
    net::TcpListener,
    os::unix::{fs::PermissionsExt, process::ExitStatusExt},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

use nix::sys::signal::Signal;
use sha2::{Digest, Sha256};

use super::{ENABLED, diagnostic_contains, external_system_policy_blocked};

/// Uncompressed size of each Node fixture's filler file. It compresses to a
/// few KiB but crosses `UNPACK_LIMIT` while being extracted.
const LARGE: usize = 8 << 20;
/// `ulimit -f` blocks (512 or 1024 bytes): far below `LARGE`, far above every
/// other file setup writes.
pub(super) const UNPACK_LIMIT: u32 = 1024;
pub(super) const OFFLINE: &[&str] = &["setup", "--tools-only", "--offline"];
const CHECK: &[&str] = &["setup", "--check", "--tools-only"];

fn archive(root: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, body) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, format!("{root}/{path}"), *body)
            .unwrap();
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) struct Run {
    pub(super) status: ExitStatus,
    pub(super) text: String,
}

/// A committed Node/pnpm repository plus an isolated home and temp dir, with
/// PATH limited to Git's directory (setup checks `.turbo/` ignore coverage).
pub(super) struct Repo {
    _temp: tempfile::TempDir,
    pub(super) root: PathBuf,
    home: PathBuf,
    pub(super) tmp: PathBuf,
    path: PathBuf,
    platform: &'static str,
    prefix: String,
    pub(super) node: Vec<u8>,
    pnpm: Vec<u8>,
}

impl Repo {
    /// None when the host's npm configuration blocks setup before any write.
    pub(super) fn new() -> Option<Self> {
        let temp = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(temp.path()).unwrap();
        let [root, home, tmp] = ["repo", "home", "tmp"].map(|name| base.join(name));
        for dir in [&root, &home, &tmp] {
            fs::create_dir_all(dir).unwrap();
        }
        let git = which::which("git").unwrap();
        let (platform, spelling) = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => ("macos-arm64", "darwin-arm64"),
            ("macos", _) => ("macos-x64", "darwin-x64"),
            ("linux", "aarch64") => ("linux-arm64-gnu", "linux-arm64"),
            _ => ("linux-x64-gnu", "linux-x64"),
        };
        let prefix = format!("node-v24.0.0-{spelling}");
        let mut repo = Self {
            _temp: temp,
            node: Vec::new(),
            pnpm: archive(
                "package",
                &[
                    (
                        "package.json",
                        br#"{"name":"pnpm","version":"10.0.0","bin":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"},"scripts":{"install":"touch hook-ran"}}"#,
                    ),
                    ("bin/pnpm.cjs", b"throw new Error('must not run');"),
                    ("bin/pnpx.cjs", b"throw new Error('must not run');"),
                ],
            ),
            path: git.parent().unwrap().to_owned(),
            root,
            home,
            tmp,
            platform,
            prefix,
        };
        for (name, contents) in [
            ("turbo.json", ENABLED),
            (".nvmrc", "24.0.0\n"),
            (".gitignore", "/.turbo/\n"),
            (
                "package.json",
                r#"{"packageManager":"pnpm@10.0.0","scripts":{"build":"touch task-ran"}}"#,
            ),
        ] {
            fs::write(repo.root.join(name), contents).unwrap();
        }
        repo.git(&["init", "-q"]);
        repo.set_node("first");
        let check = repo.run(None, CHECK);
        let code = check.status.code().unwrap();
        if external_system_policy_blocked(code, &check.text) {
            return None;
        }
        assert_eq!(code, 1, "{}", check.text);
        assert!(
            diagnostic_contains(
                &check.text,
                "managed installation is missing; run turbo setup"
            ),
            "{}",
            check.text
        );
        Some(repo)
    }

    fn git(&self, words: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(&self.root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(words)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap()
    }

    /// Commit a new locked Node artifact. Executing it would leave a marker.
    pub(super) fn set_node(&mut self, variant: &str) {
        let script = format!("#!/bin/sh\n# {variant}\ntouch probe-ran\nexit 91\n");
        self.node = archive(
            &self.prefix,
            &[
                ("bin/node", script.as_bytes()),
                ("share/large", &vec![0; LARGE]),
            ],
        );
        let lock = serde_json::json!({"schemaVersion":0,"tools":{
            "node":{"adapter":"node","version":"24.0.0","declarations":[{"file":".nvmrc","request":"24.0.0"}],
                "installation":{"kind":"managed","artifacts":{self.platform:{"distribution":{
                    "url":self.node_url(),"sha256":sha256(&self.node),
                    "format":"tar-gz","rootPrefix":self.prefix,"executables":{"node":"bin/node"}
                }}}}},
            "pnpm":{"adapter":"pnpm","version":"10.0.0","declarations":[{"file":"package.json","field":"/packageManager","request":"pnpm@10.0.0"}],
                "installation":{"kind":"managed","artifacts":{"any":{"package":{
                    "url":"https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz","sha256":sha256(&self.pnpm),
                    "format":"tar-gz","rootPrefix":"package","executables":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}
                }}}}}
        }});
        fs::write(self.root.join("turbo.lock"), lock.to_string()).unwrap();
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", variant]);
    }

    fn node_url(&self) -> String {
        format!("https://nodejs.org/dist/v24.0.0/{}.tar.gz", self.prefix)
    }

    pub(super) fn missing_node(&self) -> String {
        format!(
            "node 24.0.0 [distribution]: {} (SHA-256 {})",
            self.node_url(),
            sha256(&self.node)
        )
    }

    pub(super) fn missing_pnpm(&self) -> String {
        format!(
            "pnpm 10.0.0 [package]: https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz (SHA-256 {})",
            sha256(&self.pnpm)
        )
    }

    /// `turbo`, or `turbo` under `ulimit -f blocks`, routed through `proxy`.
    pub(super) fn command(
        &self,
        file_size_blocks: Option<u32>,
        words: &[&str],
        proxy: &TcpListener,
    ) -> Command {
        let turbo = env!("CARGO_BIN_EXE_turbo");
        let mut command = match file_size_blocks {
            None => Command::new(turbo),
            Some(blocks) => {
                let mut shell = Command::new("/bin/sh");
                shell
                    .arg("-c")
                    .arg(format!(
                        "ulimit -c 0 && ulimit -f {blocks} && exec \"$0\" \"$@\""
                    ))
                    .arg(turbo);
                shell
            }
        };
        let proxy = format!("http://{}", proxy.local_addr().unwrap());
        command
            .env_clear()
            .current_dir(&self.root)
            .args(words)
            .env("PATH", &self.path)
            .env("HOME", &self.home)
            .env("TMPDIR", &self.tmp)
            .env("XDG_CONFIG_HOME", self.home.join("config"))
            .env("TURBO_CONFIG_DIR_PATH", self.home.join("config"))
            .env("VERCEL_CONFIG_DIR_PATH", self.home.join("config"))
            .env("TURBO_TELEMETRY_DISABLED", "1")
            .env("DO_NOT_TRACK", "1")
            .env("HTTP_PROXY", &proxy)
            .env("HTTPS_PROXY", &proxy)
            .env("ALL_PROXY", &proxy)
            .env("NO_PROXY", "");
        command
    }

    /// One setup process that must make no network connection.
    pub(super) fn run(&self, file_size_blocks: Option<u32>, words: &[&str]) -> Run {
        let monitor = TcpListener::bind("127.0.0.1:0").unwrap();
        monitor.set_nonblocking(true).unwrap();
        let output = self
            .command(file_size_blocks, words, &monitor)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            matches!(monitor.accept(), Err(e) if e.kind() == io::ErrorKind::WouldBlock),
            "unexpected network connection"
        );
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Run {
            status: output.status,
            text: text.split_whitespace().collect::<Vec<_>>().join(" "),
        }
    }

    pub(super) fn expect(&self, words: &[&str], code: i32, expected: &[&str]) -> String {
        let Run { status, text } = self.run(None, words);
        assert_eq!(status.code(), Some(code), "{words:?}: {text}");
        for expected in expected {
            assert!(diagnostic_contains(&text, expected), "{expected}: {text}");
        }
        text
    }

    /// Dies with SIGXFSZ while writing the first large extracted file.
    pub(super) fn killed_unpack(&self) {
        fs::remove_dir_all(&self.tmp).unwrap();
        fs::create_dir(&self.tmp).unwrap();
        let Run { status, text } = self.run(Some(UNPACK_LIMIT), OFFLINE);
        assert_eq!(status.signal(), Some(Signal::SIGXFSZ as i32), "{text}");
        assert!(partial_scratch(&self.tmp), "kill did not land mid-unpack");
    }

    pub(super) fn cache(&self) -> PathBuf {
        let base = if cfg!(target_os = "macos") {
            self.home.join("Library/Caches")
        } else {
            self.home.join(".cache")
        };
        base.join("turborepo/setup-artifacts/v1")
    }

    /// Writes Node and pnpm entries the way an earlier setup in another clone
    /// would have.
    pub(super) fn seed(&self) -> [PathBuf; 2] {
        [(self.platform, &self.node), ("any", &self.pnpm)].map(|(platform, bytes)| {
            let path = self.cache().join(platform).join(sha256(bytes));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let _ = fs::remove_file(&path);
            fs::write(&path, bytes).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
            path
        })
    }

    /// Tracked files are unchanged and nothing (task, probe, or hook marker)
    /// appeared outside ignored `.turbo/`.
    pub(super) fn assert_committed(&self) {
        let status = self.git(&["status", "--porcelain", "--untracked-files=all"]);
        assert!(status.is_empty(), "{status}");
    }

    /// The active, verified manifest bytes, if any generation is active.
    pub(super) fn inventory(&self) -> Option<Vec<u8>> {
        let current = turborepo_tool_install::Store::inspect(&self.root).unwrap()?;
        assert_eq!(current.tools.len(), 2);
        Some(fs::read(self.root.join(".turbo/tools/manifest.json")).unwrap())
    }
}

/// An abandoned private extraction directory holding a truncated filler file.
fn partial_scratch(tmp: &Path) -> bool {
    fn truncated(path: &Path) -> bool {
        fs::read_dir(path).unwrap().any(|entry| {
            let path = entry.unwrap().path();
            if path.is_dir() {
                truncated(&path)
            } else {
                path.ends_with("share/large") && fs::metadata(&path).unwrap().len() < LARGE as u64
            }
        })
    }
    fs::read_dir(tmp).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("turborepo-archive-")
            && truncated(&path)
    })
}

#[test]
fn killed_download_releases_locks_and_offline_lists_every_missing_artifact() {
    let Some(repo) = Repo::new() else { return };
    // The only route out is a proxy that accepts and never answers.
    let stalled = TcpListener::bind("127.0.0.1:0").unwrap();
    stalled.set_nonblocking(true).unwrap();
    let log = repo.tmp.join("killed.log");
    let mut child = repo
        .command(None, &["setup", "--tools-only", "--frozen"], &stalled)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(fs::File::create(&log).unwrap())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let held = loop {
        match stalled.accept() {
            Ok((socket, _)) => break socket,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("{e}"),
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "setup exited before downloading: {}",
            fs::read_to_string(&log).unwrap()
        );
        assert!(Instant::now() < deadline, "setup never reached the network");
        std::thread::sleep(Duration::from_millis(5));
    };
    child.kill().unwrap();
    assert_eq!(child.wait().unwrap().signal(), Some(Signal::SIGKILL as i32));
    drop(held);
    repo.assert_committed();
    assert_eq!(repo.inventory(), None);
    assert!(!repo.cache().exists(), "no unverified bytes may be cached");
    repo.expect(
        CHECK,
        1,
        &["managed installation is missing; run turbo setup"],
    );
    // The dead process's writer and Store locks are gone: the retry runs now,
    // and offline names every artifact still needed.
    for _ in 0..2 {
        repo.expect(
            OFFLINE,
            1,
            &[
                "offline setup is missing verified cached artifacts; nothing was installed",
                &repo.missing_node(),
                &repo.missing_pnpm(),
                "Run turbo setup --tools-only without --offline to download and verify them.",
            ],
        );
        assert_eq!(repo.inventory(), None);
        repo.assert_committed();
    }
}

#[test]
fn killed_unpack_corrupt_cache_and_repeats_never_publish_partial_tools() {
    let Some(mut repo) = Repo::new() else { return };
    let [node, _] = repo.seed();
    repo.killed_unpack();
    assert_eq!(repo.inventory(), None);
    repo.assert_committed();
    repo.expect(
        CHECK,
        1,
        &["managed installation is missing; run turbo setup"],
    );
    // A torn cache entry is a miss, not a partial install.
    fs::remove_file(&node).unwrap();
    fs::write(&node, &repo.node[..repo.node.len() / 2]).unwrap();
    let text = repo.expect(OFFLINE, 1, &[&repo.missing_node()]);
    assert!(
        !diagnostic_contains(&text, "pnpm 10.0.0 [package]"),
        "{text}"
    );
    assert_eq!(repo.inventory(), None);
    repo.seed();
    let installed = "installed from the shared download cache";
    repo.expect(
        OFFLINE,
        0,
        &[
            &format!("node 24.0.0: {installed}"),
            &format!("pnpm 10.0.0: {installed}"),
            "Offline: used only installed tools and verified cached artifacts.",
        ],
    );
    let first = repo.inventory().unwrap();
    repo.expect(OFFLINE, 0, &["node 24.0.0: reused", "pnpm 10.0.0: reused"]);
    assert_eq!(repo.inventory().unwrap(), first);
    repo.expect(CHECK, 0, &["node 24.0.0: ready", "pnpm 10.0.0: ready"]);
    repo.assert_committed();
    // An interrupted upgrade keeps the previous complete generation active.
    repo.set_node("second");
    repo.seed();
    repo.killed_unpack();
    assert_eq!(repo.inventory().unwrap(), first);
    repo.assert_committed();
    repo.expect(
        CHECK,
        1,
        &["managed installation does not match turbo.lock; run turbo setup"],
    );
    repo.expect(OFFLINE, 0, &[&format!("node 24.0.0: {installed}")]);
    assert_ne!(repo.inventory().unwrap(), first);
    repo.expect(CHECK, 0, &["node 24.0.0: ready", "pnpm 10.0.0: ready"]);
    repo.assert_committed();
}
