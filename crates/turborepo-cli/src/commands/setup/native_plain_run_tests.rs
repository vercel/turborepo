//! Real-artifact setup -> plain CLI regression on the canonical managed stack.
//! Artifact roots and the built CLI are explicit; upstream traffic is loopback.
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
};

use sha2::Sha512;
use turborepo_run::EnvironmentVariableMap;

use super::*;

fn bundle(prefix: &str, source: &Path) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    tar.follow_symlinks(false);
    tar.append_dir_all(prefix, source).unwrap();
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gzip.write_all(&tar.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}

struct NativeArtifacts {
    node_version: String,
    manager_version: String,
    npm_version: String,
    spelling: &'static str,
    index_file: &'static str,
    node: Vec<u8>,
    manager: Vec<u8>,
}
impl NativeArtifacts {
    fn from_explicit_roots() -> Self {
        let node_root = PathBuf::from(
            std::env::var_os("TURBO_TEST_NODE_ROOT")
                .ok_or("provide real Node distribution root")
                .unwrap(),
        );
        let manager_root = PathBuf::from(
            std::env::var_os("TURBO_TEST_PNPM_ROOT")
                .ok_or("provide real pnpm package root")
                .unwrap(),
        );
        let node = Command::new(node_root.join("bin/node"))
            .env_clear()
            .args(["-p", "process.versions.node"])
            .output()
            .unwrap();
        assert!(node.status.success());
        let node_version = String::from_utf8(node.stdout).unwrap().trim().to_owned();
        let manager: Value =
            serde_json::from_slice(&fs::read(manager_root.join("package.json")).unwrap()).unwrap();
        let npm: Value = serde_json::from_slice(
            &fs::read(node_root.join("lib/node_modules/npm/package.json")).unwrap(),
        )
        .unwrap();
        let (spelling, index_file) = match provision::platform().unwrap() {
            turborepo_setup::lock::Platform::MacosArm64 => ("darwin-arm64", "osx-arm64-tar"),
            turborepo_setup::lock::Platform::MacosX64 => ("darwin-x64", "osx-x64-tar"),
            turborepo_setup::lock::Platform::LinuxX64Gnu => ("linux-x64", "linux-x64"),
            _ => ("linux-arm64", "linux-arm64"),
        };
        Self {
            node: bundle(&format!("node-v{node_version}-{spelling}"), &node_root),
            manager: bundle("package", &manager_root),
            node_version,
            manager_version: manager["version"].as_str().unwrap().into(),
            npm_version: npm["version"].as_str().unwrap().into(),
            spelling,
            index_file,
        }
    }
    fn setup(&self) -> (Fixture, LoopbackServer) {
        let f = Fixture::new();
        let root = f.owned.root();
        fs::remove_file(root.join("turbo.lock")).unwrap();
        fs::write(root.join(".nvmrc"), &self.node_version).unwrap();
        fs::write(root.join("package.json"), json!({"name":"native-fixture","packageManager":format!("pnpm@{}",self.manager_version),"scripts":{"probe":"node report.cjs","invalidate":"node invalidate.cjs"}}).to_string()).unwrap();
        fs::write(
            root.join("turbo.json"),
            json!({"futureFlags":{"experimentalSetup":true},"tasks":{
                "probe":{"cache":false,"inputs":["report.cjs"],"env":["FIXTURE_DECLARED"]},
                "invalidate":{"cache":false,"inputs":["invalidate.cjs"],"env":["FIXTURE_RECORD"]}
            }})
            .to_string(),
        )
        .unwrap();
        fs::write(root.join("report.cjs"), r#"const fs = require('fs'); const report = {node:process.version,execPath:process.execPath,manager:process.env.npm_config_user_agent,managerPath:process.env.npm_execpath,declared:process.env.FIXTURE_DECLARED,secret:process.env.FIXTURE_SECRET||null,hash:process.env.TURBO_HASH}; console.log(JSON.stringify(report)); fs.writeFileSync('result.json',JSON.stringify(report));"#).unwrap();
        fs::write(
            root.join("invalidate.cjs"),
            "require('fs').unlinkSync(process.env.FIXTURE_RECORD); console.log('trusted record \
             removed by prior task');",
        )
        .unwrap();
        let filename = format!("node-v{}-{}.tar.gz", self.node_version, self.spelling);
        let node_path = format!("/dist/v{}/{filename}", self.node_version);
        let upstream = LoopbackServer::new([
            ("/dist/index.json".into(), json!([{"version":format!("v{}",self.node_version),"npm":self.npm_version,"lts":false,"files":[self.index_file]}]).to_string().into_bytes()),
            (format!("/dist/v{}/SHASUMS256.txt",self.node_version), format!("{:x}  {filename}\n",Sha256::digest(&self.node)).into_bytes()),
            (node_path, self.node.clone()),
            (format!("/pnpm/{}",self.manager_version), json!({"name":"pnpm","version":self.manager_version,"dist":{"tarball":format!("https://registry.npmjs.org/pnpm/-/pnpm-{}.tgz",self.manager_version),"integrity":format!("sha512-{}",first_lock::base64(&Sha512::digest(&self.manager)))}}).to_string().into_bytes()),
            (format!("/pnpm/-/pnpm-{}.tgz",self.manager_version), self.manager.clone()),
        ], |_| {}).unwrap();
        let args = f.args(&["--tools-only", "--no-frozen"]);
        let result = crate::cli::dispatch_setup(&args, |args, setup| run_with_policy(args, setup,
            Some(provision::Transports {
                node: turborepo_setup::node_provision::NodeTransport::loopback_http_for_tests(upstream.origin()).unwrap(),
                pnpm: turborepo_setup::pnpm_provision::PnpmTransport::loopback_http_for_tests(upstream.origin()).unwrap(),
                registry: turborepo_setup::registry_resolution::RegistryTransport::loopback_http_for_tests(upstream.origin()).unwrap(),
                trust_base: Some(user_base(&f)),
            }), || Ok(f.owned.policy_at(&f.owned.root().join("apps/web/src"))?))).unwrap();
        assert_eq!(result.unwrap(), 0);
        assert!(root.join("turbo.lock").exists());
        (f, upstream)
    }
}

fn user_base(f: &Fixture) -> PathBuf {
    if cfg!(target_os = "macos") {
        f.owned.home().join("Library/Application Support")
    } else {
        f.owned.home().join("data")
    }
}
fn record(f: &Fixture) -> PathBuf {
    fs::read_dir(user_base(f).join("turborepo/setup-roots"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
}
fn inherited(f: &Fixture) -> EnvironmentVariableMap {
    HashMap::from([
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("HOME".into(), f.owned.home().to_string_lossy().into_owned()),
        ("FIXTURE_DECLARED".into(), "kept".into()),
        ("FIXTURE_SECRET".into(), "private".into()),
        (
            "FIXTURE_RECORD".into(),
            record(f).to_string_lossy().into_owned(),
        ),
    ])
    .into()
}

#[test]
#[ignore = "requires explicit real artifact roots and a built native turbo binary"]
fn setup_then_plain_turbo_run_uses_native_tools_and_rejects_stale_state() {
    let binary = PathBuf::from(
        std::env::var_os("TURBO_TEST_TURBO_BINARY")
            .ok_or("provide the integration binary")
            .unwrap(),
    );
    let native = NativeArtifacts::from_explicit_roots();
    let (f, upstream) = native.setup();
    let setup_hits = upstream.hits();
    for mode in ["strict", "loose"] {
        let output = Command::new(&binary)
            .current_dir(f.owned.root())
            .env_clear()
            .envs(inherited(&f).iter())
            .env("XDG_DATA_HOME", f.owned.home().join("data"))
            .env("TURBO_TELEMETRY_DISABLED", "1")
            .env("DO_NOT_TRACK", "1")
            .args([
                "run",
                "probe",
                "--env-mode",
                mode,
                "--ui=stream",
                "--no-daemon",
                "--cache=local:",
            ])
            .output()
            .unwrap();
        println!(
            "plain turbo run {mode}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value =
            serde_json::from_slice(&fs::read(f.owned.root().join("result.json")).unwrap()).unwrap();
        assert_eq!(report["node"], format!("v{}", native.node_version));
        for key in ["execPath", "managerPath"] {
            assert!(
                report[key]
                    .as_str()
                    .unwrap()
                    .starts_with(f.owned.root().join(".turbo/tools").to_str().unwrap())
            );
        }
        assert_eq!(
            report["secret"],
            if mode == "strict" {
                Value::Null
            } else {
                json!("private")
            }
        );
        assert_eq!(upstream.hits(), setup_hits);
    }
    let original_record = record(&f);
    fs::remove_file(f.owned.root().join("result.json")).unwrap();
    fs::remove_file(original_record).unwrap();
    let output = Command::new(binary)
        .current_dir(f.owned.root())
        .env_clear()
        .env("HOME", f.owned.home())
        .env("PATH", "/usr/bin:/bin")
        .env("XDG_DATA_HOME", f.owned.home().join("data"))
        .env("TURBO_TELEMETRY_DISABLED", "1")
        .env("DO_NOT_TRACK", "1")
        .args(["run", "probe", "--ui=stream", "--no-daemon"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("turbo setup"));
    assert!(!f.owned.root().join("result.json").exists());
    assert_eq!(upstream.hits(), setup_hits);
}
