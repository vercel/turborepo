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

/// Runs standalone turbo without HTTP access. Returns the exit code and
/// combined output without wrapping or gutters.
fn turbo(root: &Path, words: &[&str], path: &Path) -> (i32, String) {
    let home = tempfile::tempdir().unwrap();
    turbo_with_home(root, words, path, home.path())
}

fn turbo_with_home(root: &Path, words: &[&str], path: &Path, home: &Path) -> (i32, String) {
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    proxy.set_nonblocking(true).unwrap();
    let url = format!("http://{}", proxy.local_addr().unwrap());
    let output = Command::new(env!("CARGO_BIN_EXE_turbo"))
        .env_clear()
        .current_dir(root)
        .args(words)
        .env("PATH", path)
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("TURBO_CONFIG_DIR_PATH", home.join("config"))
        .env("VERCEL_CONFIG_DIR_PATH", home.join("config"))
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
    fs::write(
        root.join("turbo.json"),
        r#"{"futureFlags":{"experimentalSetup":true}}"#,
    )
    .unwrap();
    fs::write(root.join(".nvmrc"), "24.x").unwrap();
    // A `.git` with a HEAD makes discovery ask Git for the effective worktree.
    fs::create_dir(root.join(".git")).unwrap();
    fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();

    let text = failed_setup(root, Path::new(""));
    assert!(
        text.contains("couldnotrun`git`:itwasnotfoundonPATH"),
        "{text}"
    );
    assert!(!text.contains("(oserror2)"), "{text}");

    let bin = git_only_path();
    let text = failed_setup(root, bin.path());
    assert!(text.contains("cannotguardeffectiveGitworktree"), "{text}");
    assert!(!text.contains(".gitignore"), "{text}");
}

#[test]
fn pruned_docker_output_without_git_provisions_and_checks() {
    use std::os::unix::fs::PermissionsExt;

    use turborepo_setup::{
        lock::{Lock, Platform, Snapshot},
        node_provision::NodePlan,
        pnpm_provision::PnpmPlan,
    };

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("apps/web")).unwrap();
    for (name, contents) in [
        (
            "turbo.json",
            r#"{"futureFlags":{"experimentalSetup":true}}"#,
        ),
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
    let (platform, selector, spelling) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => (Platform::MacosArm64, "macos-arm64", "darwin-arm64"),
        ("macos", _) => (Platform::MacosX64, "macos-x64", "darwin-x64"),
        ("linux", "aarch64") => (Platform::LinuxArm64Gnu, "linux-arm64-gnu", "linux-arm64"),
        _ => (Platform::LinuxX64Gnu, "linux-x64-gnu", "linux-x64"),
    };
    let prefix = format!("node-v24.0.0-{spelling}");
    let lock = serde_json::json!({"schemaVersion":0,"tools":{
        "node":{"adapter":"node","version":"24.0.0","declarations":[{"file":".nvmrc","request":"24.0.0"}],
            "installation":{"kind":"managed","artifacts":{selector:{"distribution":{
                "url":format!("https://nodejs.org/dist/v24.0.0/{prefix}.tar.gz"),"sha256":"0".repeat(64),
                "format":"tar-gz","rootPrefix":prefix,"executables":{"node":"bin/node"}}}}}},
        "pnpm":{"adapter":"pnpm","version":"10.0.0","declarations":[{"file":"package.json","field":"/packageManager","request":"pnpm@10.0.0"}],
            "installation":{"kind":"managed","artifacts":{"any":{"package":{
                "url":"https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz","sha256":"1".repeat(64),
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

    // Seed the locked generation through the real Store so frozen setup reuses
    // it without downloads; the CLI library fixtures cover actual downloads.
    let lock = Lock::parse(lock.as_bytes()).unwrap();
    let node = NodePlan::from_lock(&lock, platform).unwrap();
    let declaration = Snapshot::capture(&root)
        .unwrap()
        .package_manager()
        .unwrap()
        .unwrap();
    let pnpm = PnpmPlan::from_declaration(&lock, platform, &node, &declaration).unwrap();
    let tools = [node.inventory_tool().clone(), pnpm.inventory_tool().clone()];
    let stage = |tool: &turborepo_tool_install::Tool, destination: &Path| {
        for relative in tool.tree_files() {
            let file = destination.join(relative);
            fs::create_dir_all(file.parent().unwrap())?;
            fs::write(&file, "#!/bin/sh\nexit 99\n")?;
            fs::set_permissions(file, fs::Permissions::from_mode(0o755))?;
        }
        Ok::<_, turborepo_tool_install::Error>(())
    };
    let mut store = turborepo_tool_install::Store::open(&root).unwrap();
    store.reconcile(&tools, stage).unwrap();
    // Inventory alone is not authority. Seed the same private, external
    // receipt that verified setup publishes, without weakening its guards.
    let home = tempfile::tempdir().unwrap();
    let records = home.path().join(if cfg!(target_os = "macos") {
        "Library/Application Support/turborepo/setup-roots"
    } else {
        "data/turborepo/setup-roots"
    });
    fs::create_dir_all(&records).unwrap();
    fs::set_permissions(&records, fs::Permissions::from_mode(0o700)).unwrap();
    let canonical_root = root.canonicalize().unwrap();
    assert!(!records.starts_with(&canonical_root));
    use std::os::unix::fs::MetadataExt;

    use sha2::{Digest, Sha256};
    let hash = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
    let identity = fs::metadata(&canonical_root).unwrap();
    let current = store.current().unwrap().unwrap();
    let record = serde_json::json!({
        "schema": 1,
        "lock_sha256": hash(&fs::read(root.join("turbo.lock")).unwrap()),
        "installation": {
            "root": canonical_root,
            "device": identity.dev(),
            "inode": identity.ino(),
            "manifest_sha256": hash(&fs::read(root.join(".turbo/tools/manifest.json")).unwrap()),
            "bin": current.bin,
            "tools": current.tools,
        },
    });
    let receipt = records.join(format!(
        "{}.json",
        hash(canonical_root.as_os_str().as_encoded_bytes())
    ));
    fs::write(&receipt, serde_json::to_vec(&record).unwrap()).unwrap();
    fs::set_permissions(&receipt, fs::Permissions::from_mode(0o600)).unwrap();
    drop(store);

    // Docker images often lack Git too; neither run needs it here.
    let frozen = ["setup", "--frozen", "--tools-only"];
    let (code, text) = turbo_with_home(&root, &frozen, Path::new(""), home.path());
    assert_eq!(code, 0, "{text}");
    assert!(
        text.contains("node24.0.0reused") && text.contains("pnpm10.0.0reused"),
        "{text}"
    );
    let (code, text) = turbo_with_home(
        &root,
        &["setup", "--check", "--tools-only"],
        Path::new(""),
        home.path(),
    );
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("node24.0.0:ready"), "{text}");

    // Once the output is a repository, setup needs Git to find the worktree
    // but no ignore rule for .turbo/.
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
    let (code, text) = turbo_with_home(&root, &frozen, Path::new(""), home.path());
    assert_eq!(code, 1, "{text}");
    assert!(
        text.contains("couldnotrun`git`:itwasnotfoundonPATH"),
        "{text}"
    );
    assert!(!root.join(".gitignore").exists());
    let (code, text) = turbo_with_home(&root, &frozen, bin.path(), home.path());
    assert_eq!(code, 0, "{text}");
    assert!(
        text.contains("node24.0.0reused") && text.contains("pnpm10.0.0reused"),
        "{text}"
    );
}
