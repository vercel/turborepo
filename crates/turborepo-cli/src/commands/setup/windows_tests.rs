#![allow(clippy::unwrap_used)]
//! Native Windows x64 setup through the real CLI path with owned fixtures:
//! verified zip Node, registry pnpm, PATHEXT exports, reuse and recovery.
use std::{fs, io::Write, path::PathBuf, process::Command};

use serde_json::{Value, json};
use sha2::{Digest, Sha256, Sha512};
use turborepo_setup::test_support::{LoopbackServer, OwnedSetupFixture};

use super::*;

const ENABLED: &str = r#"{"futureFlags":{"experimentalSetup":true}}"#;
const PREFIX: &str = "node-v24.0.0-win-x64";
const NODE_ZIP: &str = "/dist/v24.0.0/node-v24.0.0-win-x64.zip";
const FROZEN: &[&str] = &["--frozen", "--tools-only"];
const LOCAL: &[&str] = &["--tools-only", "--no-frozen"];
const CHECK: &[&str] = &["--check", "--tools-only", "--offline"];

fn system32() -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32")
}

// Stored entries from a DOS host, as in the official Windows archive.
fn zip(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let le = |out: &mut Vec<u8>, value: usize, width: usize| {
        out.extend(&(value as u32).to_le_bytes()[..width]);
    };
    let (mut local, mut central) = (Vec::new(), Vec::new());
    for (path, data) in files {
        let name = format!("{PREFIX}/{path}");
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let offset = local.len();
        for (out, signature) in [(&mut local, 0x04034b50), (&mut central, 0x02014b50)] {
            let is_central = signature == 0x02014b50;
            le(out, signature, 4);
            if is_central {
                le(out, 20, 2); // Made by DOS, version 2.0.
            }
            for (value, width) in [(20, 2), (0, 2), (0, 2), (0, 4)] {
                le(out, value, width); // Version, flags, stored, timestamp.
            }
            for value in [crc.sum() as usize, data.len(), data.len()] {
                le(out, value, 4);
            }
            le(out, name.len(), 2);
            le(out, 0, 2);
            if is_central {
                for (value, width) in [(0, 2), (0, 2), (0, 2), (0x20, 4), (offset, 4)] {
                    le(out, value, width); // Comment, disk, internal, archive bit.
                }
            }
            out.extend(name.as_bytes());
        }
        local.extend(data);
    }
    let (start, size) = (local.len(), central.len());
    local.extend(central);
    le(&mut local, 0x06054b50, 4);
    for (value, width) in [
        (0, 4),
        (files.len(), 2),
        (files.len(), 2),
        (size, 4),
        (start, 4),
    ] {
        le(&mut local, value, width);
    }
    le(&mut local, 0, 2);
    local
}

fn tgz(files: &[(&str, &str)]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, body) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, format!("package/{path}"), body.as_bytes())
            .unwrap();
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}

struct Fixture {
    owned: OwnedSetupFixture,
    node: Vec<u8>,
    pnpm: Vec<u8>,
}
impl Fixture {
    fn new(locked: bool) -> Self {
        let owned = OwnedSetupFixture::new().unwrap();
        let root = owned.root();
        owned.init_git().unwrap();
        fs::write(root.join("turbo.json"), ENABLED).unwrap();
        fs::write(root.join(".nvmrc"), "24.x\n").unwrap();
        fs::write(
            root.join("package.json"),
            r#"{"packageManager":"pnpm@10.0.0"}"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("apps/web/src")).unwrap();
        // A real PE image stands in for node.exe so activated launchers run.
        let npm = r#"{"name":"npm","version":"11.6.1","bin":{"npm":"bin/npm-cli.js","npx":"bin/npx-cli.js"}}"#;
        let node = zip(&[
            ("node.exe", fs::read(system32().join("cmd.exe")).unwrap()),
            ("npm.cmd", b"@exit /b 91\r\n".to_vec()),
            ("npx.cmd", b"@exit /b 91\r\n".to_vec()),
            ("node_modules/npm/package.json", npm.into()),
            ("node_modules/npm/bin/npm-cli.js", b"throw 1".to_vec()),
            ("node_modules/npm/bin/npx-cli.js", b"throw 1".to_vec()),
            ("resource", b"node resource".to_vec()),
        ]);
        let pnpm = tgz(&[
            (
                "package.json",
                r#"{"name":"pnpm","version":"10.0.0","bin":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"},"scripts":{"install":"echo hook > hook-ran"}}"#,
            ),
            ("bin/pnpm.cjs", "throw new Error('must not run');"),
            ("bin/pnpx.cjs", "throw new Error('must not run');"),
            ("dist/resource", "pnpm resource"),
        ]);
        let f = Self { owned, node, pnpm };
        if locked {
            f.lock();
        }
        f
    }
    fn lock(&self) {
        fs::write(self.owned.root().join(".nvmrc"), "24.0.0\n").unwrap();
        let lock = json!({"schemaVersion":0,"tools":{
            "node":{"adapter":"node","version":"24.0.0","declarations":[{"file":".nvmrc","request":"24.0.0"}],
                "installation":{"kind":"managed","artifacts":{"windows-x64":{"distribution":{
                    "url":format!("https://nodejs.org{NODE_ZIP}"),"sha256":format!("{:x}", Sha256::digest(&self.node)),
                    "format":"zip","rootPrefix":PREFIX,"executables":{"node":"node.exe"}}}}}},
            "pnpm":{"adapter":"pnpm","version":"10.0.0","declarations":[{"file":"package.json","field":"/packageManager","request":"pnpm@10.0.0"}],
                "installation":{"kind":"managed","artifacts":{"any":{"package":{
                    "url":"https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz","sha256":format!("{:x}", Sha256::digest(&self.pnpm)),
                    "format":"tar-gz","rootPrefix":"package","executables":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}}}}}}
        }});
        fs::write(self.owned.root().join("turbo.lock"), lock.to_string()).unwrap();
    }
    // Official metadata routes: only the win-x64 archive is ever served.
    fn upstream(&self) -> LoopbackServer {
        let index = json!([{"version":"v24.0.0","npm":"11.6.1","lts":"Krypton",
            "files":["osx-x64-tar","osx-arm64-tar","linux-x64","linux-arm64","win-x64-zip","win-arm64-zip"]}]);
        let mut sums = format!("{:x}  {PREFIX}.zip\n", Sha256::digest(&self.node));
        for other in [
            "darwin-x64.tar.gz",
            "darwin-arm64.tar.gz",
            "linux-x64.tar.gz",
            "linux-arm64.tar.gz",
            "win-arm64.zip",
        ] {
            let digest = Sha256::digest(other);
            sums.push_str(&format!("{digest:x}  node-v24.0.0-{other}\n"));
        }
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut integrity = String::new();
        for chunk in Sha512::digest(&self.pnpm).chunks(3) {
            let byte = |i: usize| u32::from(*chunk.get(i).unwrap_or(&0));
            let n = (byte(0) << 16) | (byte(1) << 8) | byte(2);
            for i in 0..4 {
                integrity.push(if i > chunk.len() {
                    '='
                } else {
                    ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char
                });
            }
        }
        let metadata = json!({"name":"pnpm","version":"10.0.0","dist":{
            "tarball":"https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz",
            "integrity":format!("sha512-{integrity}")}});
        LoopbackServer::new(
            [
                ("/dist/index.json".into(), index.to_string().into_bytes()),
                ("/dist/v24.0.0/SHASUMS256.txt".into(), sums.into_bytes()),
                (NODE_ZIP.into(), self.node.clone()),
                ("/pnpm/10.0.0".into(), metadata.to_string().into_bytes()),
                ("/pnpm/-/pnpm-10.0.0.tgz".into(), self.pnpm.clone()),
            ],
            |_| {},
        )
        .unwrap()
    }
    fn run(&self, upstream: &LoopbackServer, flags: &[&str]) -> Result<i32, Error> {
        let mut words = vec!["turbo".into(), "setup".into(), "--cwd".into()];
        words.push(self.owned.root().join("apps/web/src").into_os_string());
        words.extend(flags.iter().map(std::ffi::OsString::from));
        let args = Args::parse_args(words).unwrap();
        let origin = upstream.origin();
        crate::cli::dispatch_setup(&args, |args, setup_args| {
            run_with_policy(
                args,
                setup_args,
                Some(provision::Transports {
                    node: turborepo_setup::node_provision::NodeTransport::loopback_http_for_tests(origin).unwrap(),
                    pnpm: turborepo_setup::pnpm_provision::PnpmTransport::loopback_http_for_tests(origin).unwrap(),
                    registry: turborepo_setup::registry_resolution::RegistryTransport::loopback_http_for_tests(origin).unwrap(),
                    trust_base: Some(self.owned.home().to_owned()),
                    rustup: turborepo_setup::rustup_provision::RustupTransport::loopback_http_for_tests(origin).unwrap(),
                    uv: turborepo_setup::uv_resolution::UvTransport::loopback_http_for_tests(origin).unwrap(),
                    python: turborepo_setup::python_resolution::PythonTransport::loopback_http_for_tests(origin).unwrap(),
                    go: turborepo_setup::go_resolution::GoTransport::loopback_http_for_tests(origin).unwrap(),
                    bun: turborepo_setup::bun_provision::BunTransport::loopback_http_for_tests(origin).unwrap(),
                    artifact: None,
                }),
                || Ok(self.owned.policy_at(&self.owned.root().join("apps/web/src"))?),
            )
        })
        .unwrap()
    }
    fn tools(&self) -> PathBuf {
        self.owned.root().join(".turbo/tools")
    }
    fn manifest(&self) -> Vec<u8> {
        fs::read(self.tools().join("manifest.json")).unwrap()
    }
    fn plan(&self) -> turborepo_setup::activation::ActivationPlan {
        use turborepo_setup::execution_identity::{ExecutionContext, Libc};
        let context = ExecutionContext::new(
            turborepo_platform::Platform::current(),
            Libc::None,
            "host-readiness".into(),
            ["unprobed".into()].into(),
        )
        .unwrap();
        turborepo_setup::activation::ActivationPlan::inspect(self.owned.root(), context).unwrap()
    }
    fn no_execution(&self) {
        for path in ["node_modules", "hook-ran", "apps/web/src/hook-ran"] {
            assert!(!self.owned.root().join(path).exists());
        }
    }
}

fn exit_code(path: &std::ffi::OsStr, line: &str) -> Option<i32> {
    let output = Command::new(system32().join("cmd.exe"))
        .args(["/d", "/c", line])
        .env("PATH", path)
        .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
        .output()
        .unwrap();
    output.status.code()
}

/// Setup refuses Windows arm64 hosts even for this x64 build.
fn unqualified_host() -> bool {
    let unqualified = provision::windows_host_machine() != Some(0x8664);
    if unqualified {
        eprintln!("Skipped: managed setup requires a Windows x64 host.");
    }
    unqualified
}

#[test]
fn frozen_setup_installs_pathext_exports_that_activate_and_repeat_reuses() {
    if unqualified_host() {
        return;
    }
    let f = Fixture::new(true);
    let u = f.upstream();
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(
        u.requests(),
        [NODE_ZIP, "/pnpm/10.0.0", "/pnpm/-/pnpm-10.0.0.tgz"]
    );
    let plan = f.plan();
    let bin = plan.path_prepend().unwrap().to_path_buf();
    let mut names: Vec<_> = fs::read_dir(&bin)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["node.exe", "pnpm.cmd", "pnpx.cmd"]);
    let tools = bin.join("../tools");
    assert!(same_file::is_same_file(bin.join("node.exe"), tools.join("node/node.exe")).unwrap());
    assert_eq!(
        fs::read(tools.join("node/resource")).unwrap(),
        b"node resource"
    );
    assert_eq!(
        fs::read(tools.join("pnpm/dist/resource")).unwrap(),
        b"pnpm resource"
    );
    let launcher = &plan.tools()[1].executables["pnpm"];
    assert!(launcher.starts_with("managed-node-") && launcher.ends_with("/pnpm.cmd"));
    assert_eq!(
        fs::read_to_string(tools.join("pnpm").join(launcher)).unwrap(),
        "@\"%~dp0..\\..\\node\\node.exe\" \"%~dp0..\\bin\\pnpm.cjs\" %*\r\n"
    );

    // Activation prepends the ordinary (non-verbatim) bin spelling, and
    // PATHEXT reaches the managed node.exe through both launcher layers.
    let path = plan.path(Some(system32().as_os_str())).unwrap();
    let first = std::env::split_paths(&path).next().unwrap();
    assert_eq!(first, bin);
    assert!(!first.to_string_lossy().starts_with(r"\\?\"));
    assert_eq!(exit_code(&path, "pnpm /d /c exit 6"), Some(6));
    assert_eq!(exit_code(&path, "pnpx /d /c exit 7"), Some(7));
    assert_eq!(exit_code(&path, "node /d /c exit 4"), Some(4));

    let manifest = f.manifest();
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(f.run(&u, CHECK).unwrap(), 0);
    assert_eq!(u.hits(), 3);
    assert_eq!(f.manifest(), manifest);
    f.no_execution();
}

#[test]
fn local_setup_creates_portable_lock_then_frozen_and_check_reuse() {
    if unqualified_host() {
        return;
    }
    let f = Fixture::new(false);
    let u = f.upstream();
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    assert_eq!(u.hits(), 7); // Four resolution reads, then exact provisioning.
    let lock: Value =
        serde_json::from_slice(&fs::read(f.owned.root().join("turbo.lock")).unwrap()).unwrap();
    let artifacts = &lock["tools"]["node"]["installation"]["artifacts"];
    assert_eq!(artifacts["windows-x64"]["distribution"]["format"], "zip");
    assert!(artifacts["linux-x64-gnu"].is_object() && artifacts["windows-arm64"].is_object());
    let manifest = f.manifest();
    let offline = LoopbackServer::new([], |_| {}).unwrap();
    for flags in [FROZEN, LOCAL, CHECK] {
        assert_eq!(f.run(&offline, flags).unwrap(), 0);
    }
    assert_eq!(offline.hits(), 0);
    assert_eq!(f.manifest(), manifest);
    f.no_execution();
}

#[test]
fn interrupted_promotion_leftovers_and_damage_recover_through_setup() {
    if unqualified_host() {
        return;
    }
    let f = Fixture::new(true);
    let u = f.upstream();
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    let manifest = f.manifest();
    // An interrupted transaction can leave an unselected staged generation, an
    // unpersisted manifest temporary and a staged lock write behind.
    fs::create_dir_all(f.tools().join("generation-interrupted/tools/node")).unwrap();
    fs::write(
        f.tools().join("generation-interrupted/tools/node/node.exe"),
        "partial",
    )
    .unwrap();
    fs::write(f.tools().join(".tmpInterrupted"), "{\"schema\":1").unwrap();
    fs::write(
        f.owned.root().join(".turbo/setup-lock/staged"),
        "partial lock",
    )
    .unwrap();
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(f.run(&u, CHECK).unwrap(), 0);
    assert_eq!(u.hits(), 3);
    assert_eq!(f.manifest(), manifest);
    assert!(!f.owned.root().join(".turbo/setup-lock/staged").exists());

    // A damaged selected export is repaired by a complete new generation.
    let bin = f.plan().path_prepend().unwrap().to_path_buf();
    fs::remove_file(bin.join("pnpm.cmd")).unwrap();
    assert!(f.run(&u, CHECK).is_err());
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(u.hits(), 6);
    assert_ne!(f.manifest(), manifest);
    assert_eq!(f.run(&u, CHECK).unwrap(), 0);
    let path = f.plan().path(None).unwrap();
    assert_eq!(exit_code(&path, "pnpm /d /c exit 3"), Some(3));
    f.no_execution();
}
