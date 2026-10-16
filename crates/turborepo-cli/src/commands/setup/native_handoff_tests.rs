//! Actual native CLI startup after fixture setup publication, not RunBuilder
//! injection. Explicit real local artifacts; no vendor HTTP or host runtime
//! fallback.
use std::{
    os::unix::{
        fs::{PermissionsExt, symlink},
        process::CommandExt,
    },
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use super::*;

fn bundle(prefix: &str, source: &Path) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    tar.follow_symlinks(false);
    tar.append_dir_all(prefix, source).unwrap();
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gzip.write_all(&tar.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}

fn invoke(f: &Fixture, binary: &Path, cwd: &Path, words: &[&str]) -> (i32, String) {
    let log = f.owned.home().join("cli-output");
    let output = fs::File::create(&log).unwrap();
    let mut child = Command::new(binary)
        .args(if words.first() == Some(&"run") {
            vec!["--verbosity=2"]
        } else {
            vec![]
        })
        .args(words)
        .current_dir(cwd)
        .env_clear()
        .process_group(0)
        .env("HOME", f.owned.home())
        .env("XDG_DATA_HOME", f.owned.home().join("data"))
        .env("TURBO_CONFIG_DIR_PATH", f.owned.home().join("config"))
        .env("PATH", f.owned.home().join("host-bin"))
        .env("DO_NOT_TRACK", "1")
        .env("TURBO_TELEMETRY_DISABLED", "1")
        .env("TURBO_NO_UPDATE_NOTIFIER", "1")
        .env("TURBO_DOWNLOAD_LOCAL_ENABLED", "1")
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("ALL_PROXY", "http://127.0.0.1:9")
        .env("NO_PROXY", "")
        .env("HANDOFF_SECRET", "loose-only")
        .env("HANDOFF_DECLARED", "kept")
        .stdin(Stdio::null())
        .stdout(output.try_clone().unwrap())
        .stderr(output)
        .spawn()
        .unwrap();
    let started = Instant::now();
    // Keep the root unreaped until group cleanup so its PID cannot be recycled.
    let completed = loop {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let waited = unsafe {
            libc::waitid(
                libc::P_PID,
                child.id(),
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if waited != 0 {
            break false;
        }
        #[cfg(target_os = "macos")]
        let pid = info.si_pid;
        #[cfg(target_os = "linux")]
        let pid = unsafe { info.si_pid() };
        if pid != 0 {
            break true;
        }
        if started.elapsed() > Duration::from_secs(60) {
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let status = child.wait().unwrap();
    let text = fs::read_to_string(log).unwrap();
    assert!(completed, "CLI startup timed out or wait failed: {text}");
    (status.code().unwrap_or(-1), text)
}

#[test]
#[ignore = "requires explicit real Node/pnpm roots and this worktree's built TURBO_TEST_BINARY"]
fn current_native_self_local_and_copied_local_startup_preserve_root_config() {
    let binary = PathBuf::from(std::env::var_os("TURBO_TEST_BINARY").expect("built current CLI"));
    let node_root =
        PathBuf::from(std::env::var_os("TURBO_TEST_NODE_ROOT").expect("real Node root"));
    let pnpm_root =
        PathBuf::from(std::env::var_os("TURBO_TEST_PNPM_ROOT").expect("real pnpm root"));
    let native = fs::read(node_root.join("bin/node")).unwrap();
    assert!(
        native.starts_with(b"\x7fELF") || native.starts_with(b"\xcf\xfa\xed\xfe"),
        "Node must be a native binary, not a script fixture"
    );
    let version = Command::new(node_root.join("bin/node"))
        .env_clear()
        .args(["-p", "process.versions.node"])
        .output()
        .unwrap();
    assert!(version.status.success());
    let version = String::from_utf8(version.stdout).unwrap().trim().to_owned();
    let manager: Value =
        serde_json::from_slice(&fs::read(pnpm_root.join("package.json")).unwrap()).unwrap();
    assert_eq!(manager["version"], "10.0.0");
    let mut f = Fixture::new();
    let root_path = f.owned.root().to_path_buf();
    let root = root_path.as_path();
    fs::write(root.join(".nvmrc"), &version).unwrap();
    let platform = f.lock["tools"]["node"]["installation"]["artifacts"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .clone();
    let spelling = if platform == "macos-arm64" {
        "darwin-arm64"
    } else if platform == "macos-x64" {
        "darwin-x64"
    } else if platform == "linux-x64-gnu" {
        "linux-x64"
    } else {
        "linux-arm64"
    };
    let prefix = format!("node-v{version}-{spelling}");
    f.node_path = format!("/dist/v{version}/{prefix}.tar.gz");
    f.node = bundle(&prefix, &node_root);
    f.pnpm = bundle("package", &pnpm_root);
    f.lock["tools"]["node"]["version"] = json!(version);
    f.lock["tools"]["node"]["installation"]["artifacts"][&platform]["distribution"] = json!({
        "url":format!("https://nodejs.org{}",f.node_path),"sha256":format!("{:x}",Sha256::digest(&f.node)),
        "format":"tar-gz","rootPrefix":prefix,"executables":{"node":"bin/node"}});
    f.lock["tools"]["pnpm"]["installation"]["artifacts"]["any"]["package"]["sha256"] =
        json!(format!("{:x}", Sha256::digest(&f.pnpm)));
    fs::write(root.join("pnpm-workspace.yaml"), "packages:\n  - apps/*\n").unwrap();
    fs::write(
        root.join("pnpm-lock.yaml"),
        "lockfileVersion: '9.0'\nimporters:\n  .: {}\n  apps/web: {}\n",
    )
    .unwrap();
    fs::write(
        root.join("apps/web/package.json"),
        r#"{"name":"web","private":true,"scripts":{"probe":"node report.cjs"}}"#,
    )
    .unwrap();
    f.declare(json!({"name":"handoff-fixture","private":true,"packageManager":"pnpm@10.0.0"}));
    let result_file = root.join("apps/web/result.json");
    let config = |task: &str| {
        json!({"futureFlags":{"experimentalSetup":true},"agentGuidance":false,
        "noUpdateNotifier":true,"tasks":{task:{"cache":false,"inputs":["report.cjs"],"env":["HANDOFF_DECLARED"]}}}).to_string()
    };
    // Both are valid. Without an explicit file, production rejects ambiguity;
    // falling back to turbo.json must never authorize managed execution.
    fs::write(root.join("turbo.json"), r#"{"futureFlags":{"experimentalSetup":false},"agentGuidance":false,"noUpdateNotifier":true,"tasks":{"refuse":{}}}"#).unwrap();
    fs::write(root.join("turbo.jsonc"), config("probe")).unwrap();
    fs::write(root.join("apps/web/report.cjs"), r#"const fs=require('fs'),cp=require('child_process'); const p=cp.spawnSync('pnpm',['--version'],{encoding:'utf8'}); if(p.status!==0) throw Error(p.stderr); fs.writeFileSync('result.json',JSON.stringify({version:process.versions.node,v8:process.versions.v8,proof:require('crypto').createHash('sha256').update('current-native').digest('hex'),exe:process.execPath,manager:process.env.npm_execpath,pnpm:p.stdout.trim(),secret:process.env.HANDOFF_SECRET||null,declared:process.env.HANDOFF_DECLARED,hash:process.env.TURBO_HASH,args:process.argv.slice(2)}));"#).unwrap();
    let user_base = if cfg!(target_os = "macos") {
        f.owned.home().join("Library/Application Support")
    } else {
        f.owned.home().join("data")
    };
    let upstream = Upstream::new(&f, false, None);
    let selected_config = root.join("turbo.jsonc");
    let args = f.args(&[
        "--frozen",
        "--tools-only",
        "--root-turbo-json",
        selected_config.to_str().unwrap(),
    ]);
    let result = crate::cli::dispatch_setup(&args, |args, setup| run_with_policy(args, setup,
        Some(provision::Transports {
                bun: turborepo_setup::bun_provision::BunTransport::loopback_http_for_tests(upstream.origin()).unwrap(),
            artifact: None,
            trust_base:Some(user_base.clone()),
            registry:turborepo_setup::registry_resolution::RegistryTransport::loopback_http_for_tests(upstream.origin()).unwrap(),
            node:turborepo_setup::node_provision::NodeTransport::loopback_http_for_tests(upstream.origin()).unwrap(),
            pnpm:turborepo_setup::pnpm_provision::PnpmTransport::loopback_http_for_tests(upstream.origin()).unwrap(),
            uv:turborepo_setup::uv_resolution::UvTransport::loopback_http_for_tests(upstream.origin()).unwrap(),
            rustup:turborepo_setup::rustup_provision::RustupTransport::loopback_http_for_tests(upstream.origin()).unwrap(),
            python:turborepo_setup::python_resolution::PythonTransport::loopback_http_for_tests(upstream.origin()).unwrap(),
            go:turborepo_setup::go_resolution::GoTransport::loopback_http_for_tests(upstream.origin()).unwrap(),
        }), || Ok(f.owned.policy_at(&root.join("apps/web/src"))?))).unwrap();
    assert_eq!(result.unwrap(), 0);
    let host_bin = f.owned.home().join("host-bin");
    fs::create_dir(&host_bin).unwrap();
    symlink("/bin/sh", host_bin.join("sh")).unwrap();
    symlink(which::which("git").unwrap(), host_bin.join("git")).unwrap();
    for tool in ["node", "pnpm", "npm", "npx", "corepack"] {
        let file = host_bin.join(tool);
        fs::write(
            &file,
            format!(
                "#!/bin/sh\nprintf '%s' '{tool}' > '{}'; exit 97\n",
                f.owned.home().join("host-tool-ran").display()
            ),
        )
        .unwrap();
        fs::set_permissions(file, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let package = root
        .join("node_modules")
        .join(turborepo_shim::TurboState::scoped_platform_package_scope())
        .join(turborepo_shim::TurboState::scoped_platform_package_dir());
    fs::create_dir_all(package.join("bin")).unwrap();
    fs::write(
        package.join("package.json"),
        json!({"version":crate::get_version()}).to_string(),
    )
    .unwrap();
    let local = package.join("bin/turbo");
    let tools_before = publication_guards::state(&root.join(".turbo/tools"));
    let lock_before = fs::read(root.join("turbo.lock")).unwrap();
    for copied in [false, true] {
        if copied {
            fs::copy(&binary, &local).unwrap();
        } else {
            symlink(&binary, &local).unwrap();
        }
        let entry = if copied { &binary } else { &local };
        let (code, query) = invoke(
            &f,
            &local,
            root,
            &[turborepo_shim::capabilities::QUERY_FLAG],
        );
        assert_eq!(code, 0, "{query}");
        let report = turborepo_shim::capabilities::Capabilities::parse(query.as_bytes()).unwrap();
        assert!(!report.supports_schema(0));
        let requirement = turborepo_shim::capabilities::Requirement::new(0, ["node", "pnpm"]);
        assert_eq!(report.shortfall(&requirement), None);
        for mode in ["strict", "loose"] {
            let mode_arg = format!("--env-mode={mode}");
            // Relative path is invocation-relative, then crosses a root-cwd handoff.
            let (code, text) = invoke(
                &f,
                entry,
                &root.join("apps/web/src"),
                &[
                    "run",
                    "probe",
                    "--root-turbo-json=../../../turbo.jsonc",
                    &mode_arg,
                    "--ui=stream",
                    "--cache=local:",
                    "--",
                    "argument with spaces",
                ],
            );
            assert_eq!(code, 0, "copied={copied} {mode}: {text}");
            let dispatched = format!(
                "Running local turbo binary in {}",
                local.canonicalize().unwrap().display()
            );
            assert_eq!(
                text.contains(&dispatched),
                copied,
                "copied={copied}: wrong CLI dispatch path\n{text}"
            );
            let result: Value =
                serde_json::from_slice(&fs::read(&result_file).unwrap_or_else(|e| {
                    panic!("copied={copied} {mode}: task output missing: {e}\n{text}")
                }))
                .unwrap();
            assert_eq!(result["version"], version);
            assert!(!result["v8"].as_str().unwrap().is_empty());
            assert_eq!(
                result["proof"],
                format!("{:x}", Sha256::digest(b"current-native"))
            );
            assert_eq!(result["pnpm"], "10.0.0");
            for key in ["exe", "manager"] {
                assert!(
                    Path::new(result[key].as_str().unwrap()).starts_with(root.join(".turbo/tools")),
                    "{result}"
                );
            }
            assert_eq!(result["declared"], "kept");
            assert_eq!(
                result["secret"],
                if mode == "loose" {
                    json!("loose-only")
                } else {
                    Value::Null
                }
            );
            assert!(!result["hash"].as_str().unwrap().is_empty());
            assert_eq!(result["args"], json!(["argument with spaces"]));
            println!("PASS copied={copied} mode={mode}: {result}");
            fs::remove_file(&result_file).unwrap();
        }
        // A same-build direct local invocation must also avoid recursion.
        let (code, text) = invoke(
            &f,
            &local,
            root,
            &[
                "run",
                "probe",
                "--root-turbo-json=turbo.jsonc",
                "--ui=stream",
                "--cache=local:",
            ],
        );
        assert_eq!(code, 0, "{text}");
        let marker = fs::read(&result_file).unwrap();
        fs::write(root.join(".nvmrc"), "26.0.0").unwrap();
        let (code, text) = invoke(
            &f,
            entry,
            root,
            &[
                "run",
                "probe",
                "--root-turbo-json=turbo.jsonc",
                "--ui=stream",
                "--cache=local:",
            ],
        );
        assert_ne!(code, 0);
        assert!(text.contains("turbo setup"), "{text}");
        assert_eq!(fs::read(&result_file).unwrap(), marker);
        fs::write(root.join(".nvmrc"), &version).unwrap();
        fs::remove_file(&result_file).unwrap();
        fs::remove_file(&local).unwrap();
    }
    // An installed package whose native binary disappeared is not an absent pin.
    let (code, text) = invoke(
        &f,
        &binary,
        root,
        &[
            "run",
            "probe",
            "--root-turbo-json=turbo.jsonc",
            "--ui=stream",
            "--cache=local:",
        ],
    );
    assert_ne!(code, 0);
    assert!(text.contains("installed native CLI is damaged"), "{text}");
    assert!(
        !result_file.exists(),
        "damaged pin was replaced by the current CLI"
    );
    assert_eq!(
        publication_guards::state(&root.join(".turbo/tools")),
        tools_before
    );
    assert_eq!(fs::read(root.join("turbo.lock")).unwrap(), lock_before);
    assert!(!f.owned.home().join("host-tool-ran").exists());
    assert_eq!(upstream.hits(), 3);
}
