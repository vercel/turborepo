//! Native Windows x64 dependency installs through the real setup path: each
//! manager's frozen command through its Windows entrypoint (`npm.cmd`,
//! `pnpm.cmd` and `yarn.cmd` launchers on the managed node.exe, `bun.exe`),
//! lifecycle skip flags, exit-status pass-through, and restored lockfiles.
//! Every fixture tool is the host's node.exe running a recording script.
use super::*;

/// Appends `<image> <entry> <arguments>` to `installs` in the repository
/// root. Then exits 3 when `stale` exists, or rewrites the lockfile that
/// `rewrite` names.
const RECORD: &str = r#"
const fs = require('fs'), path = require('path');
const [entry, ...args] = process.argv.slice(1);
const words = [path.basename(process.execPath), path.basename(entry), ...args];
fs.appendFileSync('installs', words.join(' ') + '\n');
if (fs.existsSync('stale')) process.exit(3);
if (fs.existsSync('rewrite')) fs.writeFileSync(fs.readFileSync('rewrite', 'utf8'), 'rewritten');
fs.mkdirSync('node_modules', { recursive: true });
"#;
const NPM_CMD: &[u8] = b"@\"%~dp0node.exe\" \"%~dp0node_modules\\npm\\bin\\npm-cli.js\" %*\r\n";
const BUN: &str = "/oven-sh/bun/releases/download/bun-v1.2.23";

/// A repository whose package manager is `manager` (with its committed
/// `lockfile`), served by fixtures whose executables record installs.
fn fixture(node: &[u8], manager: &str, lockfile: &str) -> (Fixture, LoopbackServer) {
    let mut f = Fixture::new(false);
    let npm = r#"{"name":"npm","version":"11.6.1","bin":{"npm":"bin/npm-cli.js","npx":"bin/npx-cli.js"}}"#;
    f.node = zip(&[
        ("node.exe", node.to_vec()),
        ("npm.cmd", NPM_CMD.to_vec()),
        ("npx.cmd", b"@exit /b 91\r\n".to_vec()),
        ("node_modules/npm/package.json", npm.into()),
        ("node_modules/npm/bin/npm-cli.js", RECORD.into()),
        ("node_modules/npm/bin/npx-cli.js", RECORD.into()),
    ]);
    f.pnpm = tgz(&[
        (
            "package.json",
            r#"{"name":"pnpm","version":"10.0.0","bin":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}}"#,
        ),
        ("bin/pnpm.cjs", RECORD),
        ("bin/pnpx.cjs", RECORD),
    ]);
    let root = f.owned.root();
    let manifest = json!({ "packageManager": manager }).to_string();
    fs::write(root.join("package.json"), manifest).unwrap();
    fs::write(root.join(lockfile), "committed lock\n").unwrap();
    let mut routes = Vec::new();
    let (name, version) = manager.split_once('@').unwrap();
    if name == "yarn" {
        let package = if version.starts_with("1.") {
            "yarn"
        } else {
            "@yarnpkg/cli-dist"
        };
        let manifest = json!({"name":package,"version":version,
            "bin":{"yarn":"./bin/yarn.js","yarnpkg":"./bin/yarn.js"}});
        let bytes = tgz(&[
            ("package.json", &manifest.to_string()),
            ("bin/yarn.js", RECORD),
        ]);
        let tarball = format!(
            "/{package}/-/{}-{version}.tgz",
            package.rsplit('/').next().unwrap()
        );
        let metadata = json!({"name":package,"version":version,"dist":{
            "tarball":format!("https://registry.npmjs.org{tarball}"),"integrity":integrity(&bytes)}});
        routes.push((
            format!("/{package}/{version}"),
            metadata.to_string().into_bytes(),
        ));
        routes.push((tarball, bytes));
    }
    if name == "bun" {
        // As bun.exe, node.exe takes Bun's `install` argument as its entry
        // point: the repository's `install` script records the invocation.
        fs::write(root.join("install"), RECORD).unwrap();
        let archive = zip_in(
            "bun-windows-x64-baseline",
            true,
            &[("bun.exe", node.to_vec())],
        );
        let sums = format!(
            "{:x}  bun-windows-x64-baseline.zip\n",
            Sha256::digest(&archive)
        );
        routes.push((format!("{BUN}/SHASUMS256.txt"), sums.into_bytes()));
        routes.push((format!("{BUN}/bun-windows-x64-baseline.zip"), archive));
    }
    let u = f.upstream_with(routes);
    (f, u)
}

fn installs(f: &Fixture) -> Vec<String> {
    let installs = fs::read_to_string(f.owned.root().join("installs")).unwrap_or_default();
    installs.lines().map(str::to_owned).collect()
}

fn host_node() -> Vec<u8> {
    fs::read(which::which("node").unwrap()).unwrap()
}

#[test]
fn each_manager_installs_through_its_windows_entrypoint_and_repeats() {
    if unqualified_host() {
        return;
    }
    let node = host_node();
    for (manager, lockfile, command, skip) in [
        (
            "npm@11.6.1",
            "package-lock.json",
            "node.exe npm-cli.js ci",
            "--ignore-scripts",
        ),
        (
            "pnpm@10.0.0",
            "pnpm-lock.yaml",
            "node.exe pnpm.cjs install --frozen-lockfile",
            "--ignore-scripts",
        ),
        (
            "yarn@1.22.22",
            "yarn.lock",
            "node.exe yarn.js install --frozen-lockfile",
            "--ignore-scripts",
        ),
        (
            "yarn@4.5.0",
            "yarn.lock",
            "node.exe yarn.js install --immutable",
            "--mode=skip-build",
        ),
        (
            "bun@1.2.23",
            "bun.lock",
            "bun.exe install --frozen-lockfile",
            "--ignore-scripts",
        ),
    ] {
        let (f, u) = fixture(&node, manager, lockfile);
        let root = f.owned.root();
        // Local setup locks and installs tools, then dependencies; repeats
        // rerun the native command, with the manager's own skip flag.
        assert_eq!(f.run(&u, &["--no-frozen"]).unwrap(), 0, "{manager}");
        fs::remove_dir_all(root.join("node_modules")).unwrap();
        assert_eq!(f.run(&u, &["--frozen"]).unwrap(), 0, "{manager}");
        assert!(root.join("node_modules").is_dir(), "{manager}");
        let skipped = ["--frozen", "--ignore-scripts"];
        assert_eq!(f.run(&u, &skipped).unwrap(), 0, "{manager}");
        let expected = [command.into(), command.into(), format!("{command} {skip}")];
        assert_eq!(installs(&f), expected, "{manager}");
        assert_eq!(fs::read(root.join(lockfile)).unwrap(), b"committed lock\n");
        // Tools-only setup never starts an installer.
        assert_eq!(f.run(&u, FROZEN).unwrap(), 0, "{manager}");
        assert_eq!(installs(&f).len(), 3, "{manager}");
    }
}

#[test]
fn installer_statuses_pass_through_and_rewritten_lockfiles_are_restored() {
    if unqualified_host() {
        return;
    }
    let node = host_node();
    for (manager, committed, written) in [
        ("pnpm@10.0.0", "pnpm-lock.yaml", "pnpm-lock.yaml"),
        // Bun's text lock written beside a committed binary one is removed.
        ("bun@1.2.23", "bun.lockb", "bun.lock"),
    ] {
        let (f, u) = fixture(&node, manager, committed);
        let root = f.owned.root();
        assert_eq!(f.run(&u, &["--no-frozen", "--tools-only"]).unwrap(), 0);
        fs::write(root.join("stale"), "").unwrap();
        assert_eq!(f.run(&u, &["--frozen"]).unwrap(), 3, "{manager}");
        fs::remove_file(root.join("stale")).unwrap();
        fs::write(root.join("rewrite"), written).unwrap();
        let error = f.run(&u, &["--frozen"]).unwrap_err().to_string();
        assert!(error.contains("it was restored"), "{manager}: {error}");
        assert_eq!(fs::read(root.join(committed)).unwrap(), b"committed lock\n");
        assert_eq!(root.join(written).exists(), written == committed);
        fs::remove_file(root.join("rewrite")).unwrap();
        assert_eq!(f.run(&u, &["--frozen"]).unwrap(), 0, "{manager}");
        assert_eq!(installs(&f).len(), 3, "{manager}");
    }
}
