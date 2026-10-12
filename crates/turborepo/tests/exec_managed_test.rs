//! Real `turbo exec` after fixture-provisioned managed Node/pnpm. Loopback
//! artifacts only: no vendor network, host tools, installs, or lock writes.
#![cfg(all(
    unix,
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    io::Write,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Command, Output},
};

use sha2::{Digest, Sha256};
use turborepo_setup::test_support::{LoopbackServer, OwnedSetupFixture};

/// Records how it was started outside the repository, then exits 7.
const NODE: &str = "#!/bin/sh\npwd > \"$HOME/exec-record\"\nprintf '%s\\n' \"$0\" \"$@\" >> \
                    \"$HOME/exec-record\"\nexit 7\n";

fn archive(root: &str, files: &[(&str, &str)]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, body) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, format!("{root}/{path}"), body.as_bytes())
            .unwrap();
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct Repo {
    owned: OwnedSetupFixture,
    upstream: LoopbackServer,
}

impl Repo {
    /// `turbo setup --frozen --tools-only` from a nested directory.
    fn provisioned() -> Self {
        let owned = OwnedSetupFixture::new().unwrap();
        let root = owned.root();
        owned.init_git().unwrap();
        let config = r#"{"futureFlags":{"experimentalSetup":true},"noUpdateNotifier":true,"agentGuidance":false}"#;
        fs::write(root.join("turbo.json"), config).unwrap();
        fs::write(root.join(".nvmrc"), "24.0.0\n").unwrap();
        let package = r#"{"name":"fixture","private":true,"packageManager":"pnpm@10.0.0"}"#;
        fs::write(root.join("package.json"), package).unwrap();
        fs::create_dir_all(root.join("apps/web/src")).unwrap();
        let (platform, spelling) = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => ("macos-arm64", "darwin-arm64"),
            ("macos", _) => ("macos-x64", "darwin-x64"),
            ("linux", "aarch64") => ("linux-arm64-gnu", "linux-arm64"),
            _ => ("linux-x64-gnu", "linux-x64"),
        };
        let prefix = format!("node-v24.0.0-{spelling}");
        let node_path = format!("/dist/v24.0.0/{prefix}.tar.gz");
        let node = archive(&prefix, &[("bin/node", NODE)]);
        let pnpm = archive(
            "package",
            &[
                (
                    "package.json",
                    r#"{"name":"pnpm","version":"10.0.0","bin":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}}"#,
                ),
                ("bin/pnpm.cjs", "throw new Error('fixture');"),
                ("bin/pnpx.cjs", "throw new Error('fixture');"),
            ],
        );
        let lock = serde_json::json!({"schemaVersion":0,"tools":{
            "node":{"adapter":"node","version":"24.0.0","declarations":[{"file":".nvmrc","request":"24.0.0"}],
                "installation":{"kind":"managed","artifacts":{platform:{"distribution":{
                    "url":format!("https://nodejs.org{node_path}"),"sha256":sha256(&node),
                    "format":"tar-gz","rootPrefix":prefix,"executables":{"node":"bin/node"}}}}}},
            "pnpm":{"adapter":"pnpm","version":"10.0.0","declarations":[{"file":"package.json","field":"/packageManager","request":"pnpm@10.0.0"}],
                "installation":{"kind":"managed","artifacts":{"any":{"package":{
                    "url":"https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz","sha256":sha256(&pnpm),
                    "format":"tar-gz","rootPrefix":"package","executables":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}}}}}}
        }});
        fs::write(root.join("turbo.lock"), lock.to_string()).unwrap();
        let upstream = LoopbackServer::node_pnpm(node_path, node, pnpm, false, |_| {}).unwrap();
        let invocation = root.join("apps/web/src");
        let user_base = if cfg!(target_os = "macos") {
            owned.home().join("Library/Application Support")
        } else {
            owned.home().join("data")
        };
        let code = turborepo_cli::test_support::frozen_tools_for_tests(
            &invocation,
            &user_base,
            upstream.origin(),
            || owned.policy_at(&invocation),
        )
        .unwrap();
        assert_eq!(code, 0);
        // Host tools of the same name would leave a marker if exec fell back.
        let host = owned.home().join("host-bin");
        fs::create_dir(&host).unwrap();
        symlink(which::which("git").unwrap(), host.join("git")).unwrap();
        for tool in ["node", "pnpm"] {
            let marker = owned.home().join("host-ran");
            let script = format!("#!/bin/sh\n: > '{}'\nexit 97\n", marker.display());
            fs::write(host.join(tool), script).unwrap();
            fs::set_permissions(host.join(tool), fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self { owned, upstream }
    }

    fn exec(&self, dir: &str, words: &[&str]) -> (Output, String) {
        let home = self.owned.home();
        let output = Command::new(env!("CARGO_BIN_EXE_turbo"))
            .current_dir(self.owned.root().join(dir))
            .args(words)
            .env_clear()
            .env("HOME", home)
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("TURBO_CONFIG_DIR_PATH", home.join("config"))
            .env("PATH", home.join("host-bin"))
            .env("TURBO_NO_UPDATE_NOTIFIER", "1")
            .env("TURBO_TELEMETRY_DISABLED", "1")
            .env("DO_NOT_TRACK", "1")
            .env("HTTP_PROXY", "http://127.0.0.1:9")
            .env("HTTPS_PROXY", "http://127.0.0.1:9")
            .env("ALL_PROXY", "http://127.0.0.1:9")
            .output()
            .unwrap();
        let record = home.join("exec-record");
        let text = fs::read_to_string(&record).unwrap_or_default();
        let _ = fs::remove_file(record);
        (output, text)
    }

    fn state(&self) -> (Vec<u8>, Vec<u8>) {
        let root = self.owned.root();
        (
            fs::read(root.join("turbo.lock")).unwrap(),
            fs::read(root.join(".turbo/tools/manifest.json")).unwrap(),
        )
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn exec_runs_managed_tools_from_root_nested_and_cwd_without_installing() {
    let repo = Repo::provisioned();
    let root = repo.owned.root().to_owned();
    let (before, hits) = (repo.state(), repo.upstream.hits());
    let tail = ["--", "node", "a b", "", "--", "--cwd", "*"];
    for (dir, flags, cwd) in [
        ("", &[][..], root.clone()),
        ("apps/web/src", &[], root.join("apps/web/src")),
        ("", &["--cwd", "apps/web"], root.join("apps/web")),
    ] {
        let words: Vec<_> = ["exec"].iter().chain(flags).chain(&tail).copied().collect();
        let (output, record) = repo.exec(dir, &words);
        assert_eq!(
            output.status.code(),
            Some(7),
            "{words:?}: {}",
            stderr(&output)
        );
        assert!(output.stdout.is_empty(), "exec must not print to stdout");
        let lines: Vec<_> = record.lines().collect();
        assert_eq!(PathBuf::from(lines[0]).canonicalize().unwrap(), cwd);
        assert!(Path::new(lines[1]).starts_with(&root), "{}", lines[1]);
        assert_eq!(&lines[2..], ["a b", "", "--", "--cwd", "*"]);
    }
    // Managed pnpm dispatches through the managed Node runtime.
    let (output, record) = repo.exec("apps/web", &["exec", "--", "pnpm", "--version"]);
    assert_eq!(output.status.code(), Some(7), "{}", stderr(&output));
    let lines: Vec<_> = record.lines().collect();
    assert!(Path::new(lines[1]).starts_with(&root));
    assert!(lines[2].ends_with("pnpm.cjs"), "{record}");
    assert_eq!(lines[3], "--version");

    assert!(!repo.owned.home().join("host-ran").exists());
    assert_eq!(repo.upstream.hits(), hits, "exec must not download");
    assert_eq!(
        repo.state(),
        before,
        "exec must not install or write the lock"
    );
}

#[test]
fn exec_fails_after_drift_or_damage_and_runs_nothing() {
    let repo = Repo::provisioned();
    let root = repo.owned.root().to_owned();
    fs::write(root.join(".nvmrc"), "26.0.0\n").unwrap();
    let (output, record) = repo.exec("apps/web", &["exec", "--", "node"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("turbo setup"),
        "{}",
        stderr(&output)
    );
    assert!(record.is_empty());

    fs::write(root.join(".nvmrc"), "24.0.0\n").unwrap();
    let current = turborepo_tool_install::Store::inspect(&root)
        .unwrap()
        .unwrap();
    fs::remove_file(current.bin.join("pnpm")).unwrap();
    let (output, record) = repo.exec("", &["exec", "--", "node"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("turbo setup"),
        "{}",
        stderr(&output)
    );
    assert!(record.is_empty());
    assert!(!repo.owned.home().join("host-ran").exists());
}
