//! Setup explains missing or unusable Git inside repositories before any
//! writer, and needs no Git outside any repository (pruned Docker output).
#![cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
#![allow(clippy::unwrap_used)]

use std::{
    fs, io,
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
};

fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = vec![];
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(snapshot(&path));
            files.push((path, vec![]));
        } else {
            let bytes = fs::read(&path).unwrap();
            files.push((path, bytes));
        }
    }
    files.sort();
    files
}

/// Stack B (#14494) keeps setup user state outside the repository; one home
/// per test process (nextest runs each test in its own process).
fn home() -> &'static Path {
    static HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    HOME.get_or_init(|| tempfile::tempdir().unwrap()).path()
}

/// A gzipped tar of executable `files` under `root`.
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
    io::Write::write_all(&mut gzip, &tar.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}

/// Runs standalone turbo without HTTP access. Returns the exit code and
/// combined output without wrapping or gutters.
fn turbo(root: &Path, words: &[&str], path: &Path) -> (i32, String) {
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    proxy.set_nonblocking(true).unwrap();
    let url = format!("http://{}", proxy.local_addr().unwrap());
    let output = Command::new(env!("CARGO_BIN_EXE_turbo"))
        .env_clear()
        .current_dir(root)
        .args(words)
        .env("PATH", path)
        .env("HOME", home())
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("TURBO_CONFIG_DIR_PATH", root.join("config"))
        .env("VERCEL_CONFIG_DIR_PATH", root.join("config"))
        .env("TURBO_TELEMETRY_DISABLED", "1")
        .env("DO_NOT_TRACK", "1")
        .env("HTTP_PROXY", &url)
        .env("HTTPS_PROXY", &url)
        .env("ALL_PROXY", &url)
        .env("NO_PROXY", "")
        .output()
        .unwrap();
    assert!(
        matches!(proxy.accept(), Err(e) if e.kind() == io::ErrorKind::WouldBlock),
        "unexpected HTTP connection"
    );
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (
        output.status.code().unwrap(),
        text.replace([' ', '|', '\n'], ""),
    )
}

/// Runs local first-lock setup, which must fail without writing.
fn failed_setup(root: &Path, path: &Path) -> String {
    let before = snapshot(root);
    let (code, text) = turbo(root, &["setup", "--tools-only", "--no-frozen"], path);
    assert_eq!(code, 1, "{text}");
    assert_eq!(snapshot(root), before, "setup must not write");
    text
}

/// Strict source policy may stop on host System npm configuration (as on
/// macOS runners) before Git is consulted. That is reported, never bypassed.
fn system_policy_blocked(text: &str) -> bool {
    let blocked = text.contains("official-onlysetuprejectsSystemconfiguration")
        || text.contains("cannotsafelyinspectSystemconfiguration");
    if blocked {
        eprintln!("Git diagnostics not reached: blocked by host System npm configuration.");
    }
    blocked
}

/// A directory containing only the real Git executable.
fn git_only_path() -> tempfile::TempDir {
    let bin = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(which::which("git").unwrap(), bin.path().join("git")).unwrap();
    bin
}

#[test]
fn missing_git_and_unusable_repositories_get_actionable_errors() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("turbo.json"), "{}").unwrap();
    fs::write(root.join(".nvmrc"), "24.x").unwrap();
    // A `.git` Git cannot use still marks a repository: the checks stay strict.
    fs::create_dir(root.join(".git")).unwrap();

    let text = failed_setup(root, Path::new(""));
    if system_policy_blocked(&text) {
        return;
    }
    assert!(
        text.contains("couldnotrun`git`:itwasnotfoundonPATH"),
        "{text}"
    );
    assert!(!text.contains("(oserror2)"), "{text}");

    // With Git available, an unusable repository is a Git failure, not a
    // missing ignore rule that editing .gitignore could fix.
    let bin = git_only_path();
    let text = failed_setup(root, bin.path());
    assert!(
        text.contains("Gitcouldnotcheckwhether.turbo/isignored"),
        "{text}"
    );
    assert!(!text.contains(".gitignore"), "{text}");
}

#[test]
fn pruned_docker_output_without_git_provisions_and_checks() {
    use std::os::unix::fs::PermissionsExt;

    use sha2::Digest;

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("apps/web")).unwrap();
    for (name, contents) in [
        ("turbo.json", "{}"),
        (".nvmrc", "24.0.0"),
        (
            "package.json",
            r#"{"name":"fixture","packageManager":"pnpm@10.0.0","workspaces":["apps/*"]}"#,
        ),
        ("pnpm-workspace.yaml", "packages: ['apps/*']\n"),
        (
            "pnpm-lock.yaml",
            "lockfileVersion: '9.0'\nimporters:\n  .: {}\n  apps/web: {}\n",
        ),
        ("apps/web/package.json", r#"{"name":"web"}"#),
    ] {
        fs::write(source.join(name), contents).unwrap();
    }
    let (selector, spelling) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => ("macos-arm64", "darwin-arm64"),
        ("macos", _) => ("macos-x64", "darwin-x64"),
        ("linux", "aarch64") => ("linux-arm64-gnu", "linux-arm64"),
        _ => ("linux-x64-gnu", "linux-x64"),
    };
    let prefix = format!("node-v24.0.0-{spelling}");
    // Real locked archives: #14494 reuses only generations it recorded, so
    // setup must install them itself (from the shared download cache below).
    let node = archive(&prefix, &[("bin/node", "#!/bin/sh\nexit 99\n")]);
    let pnpm = archive(
        "package",
        &[
            (
                "package.json",
                r#"{"name":"pnpm","version":"10.0.0","bin":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}}"#,
            ),
            ("bin/pnpm.cjs", "throw new Error('must not run');"),
            ("bin/pnpx.cjs", "throw new Error('must not run');"),
        ],
    );
    let sha256 = |bytes: &[u8]| format!("{:x}", sha2::Sha256::digest(bytes));
    let lock = serde_json::json!({"schemaVersion":0,"tools":{
        "node":{"adapter":"node","version":"24.0.0","declarations":[{"file":".nvmrc","request":"24.0.0"}],
            "installation":{"kind":"managed","artifacts":{selector:{"distribution":{
                "url":format!("https://nodejs.org/dist/v24.0.0/{prefix}.tar.gz"),"sha256":sha256(&node),
                "format":"tar-gz","rootPrefix":prefix,"executables":{"node":"bin/node"}}}}}},
        "pnpm":{"adapter":"pnpm","version":"10.0.0","declarations":[{"file":"package.json","field":"/packageManager","request":"pnpm@10.0.0"}],
            "installation":{"kind":"managed","artifacts":{"any":{"package":{
                "url":"https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz","sha256":sha256(&pnpm),
                "format":"tar-gz","rootPrefix":"package","executables":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}}}}}}
    }}).to_string();
    fs::write(source.join("turbo.lock"), &lock).unwrap();

    let out = temp.path().join("out");
    let out_dir = out.to_str().unwrap();
    let (code, text) = turbo(
        &source,
        &["prune", "web", "--docker", "--out-dir", out_dir],
        Path::new(""),
    );
    assert_eq!(code, 0, "{text}");
    let root = out.join("full");
    assert!(turborepo_setup::writer_storage::outside_git_repository(&root).unwrap());

    // Seed the shared download cache the way setup in another clone would, so
    // frozen setup installs verified bytes without traffic and records trust;
    // the repeat reuses them.
    let cache = if cfg!(target_os = "macos") {
        home().join("Library/Caches")
    } else {
        home().join(".cache")
    }
    .join("turborepo/setup-artifacts/v1");
    for (platform, bytes) in [(selector, &node), ("any", &pnpm)] {
        let path = cache.join(platform).join(sha256(bytes));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    }

    // Docker images often lack Git too; neither run needs it here.
    let frozen = ["setup", "--frozen", "--tools-only"];
    for outcome in ["installedfromtheshareddownloadcache", "reused"] {
        let (code, text) = turbo(&root, &frozen, Path::new(""));
        if system_policy_blocked(&text) {
            return;
        }
        assert_eq!(code, 0, "{text}");
        assert!(
            text.contains(&format!("node24.0.0:{outcome}"))
                && text.contains(&format!("pnpm10.0.0:{outcome}")),
            "{text}"
        );
        assert!(text.contains("NoGitrepositoryfound"), "{text}");
    }
    let (code, text) = turbo(&root, &["setup", "--check", "--tools-only"], Path::new(""));
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("node24.0.0:ready"), "{text}");

    // Once the output is a repository, the strict checks apply again.
    let bin = git_only_path();
    assert!(
        Command::new("git")
            .current_dir(&root)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .args(["init", "-q"])
            .status()
            .unwrap()
            .success()
    );
    let (code, text) = turbo(&root, &frozen, Path::new(""));
    assert_eq!(code, 1, "{text}");
    assert!(
        text.contains("couldnotrun`git`:itwasnotfoundonPATH"),
        "{text}"
    );
    let (code, text) = turbo(&root, &frozen, bin.path());
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("add/.turbo/to.gitignore"), "{text}");
}
