//! Native Windows x64 Go through the real CLI path: the official zip layout,
//! `.exe` exports, a portable lock, offline repeats, and the managed run and
//! exec environment. Local fixtures only.
use turborepo_setup::{go_resolution::INDEX_PATH, test_support::stored_zip_entries};

use super::*;

const GO: &str = r#"{"futureFlags":{"experimentalSetup":true,"experimentalGoWorkspaces":true}}"#;
const ZIP: &str = "/go/go1.22.12.windows-amd64.zip";

/// The official layout, including a UTF-8 named `go/test` fixture that setup
/// must never write. Real PE images stand in for the executables.
fn go_zip() -> Vec<u8> {
    let image = fs::read(system32().join("cmd.exe")).unwrap();
    stored_zip_entries(&[
        ("go/bin/go.exe", &image[..]),
        ("go/bin/gofmt.exe", &image[..]),
        ("go/src/go.mod", &b"module std\n"[..]),
        (
            "go/test/fixedbugs/issue27836.dir/\u{de}foo.go",
            &b"package p\n"[..],
        ),
    ])
}

/// The index lists every lockable target; only the windows-amd64 zip is served.
fn go_upstream(zip: &[u8]) -> LoopbackServer {
    let files: Vec<_> = ["darwin-amd64", "darwin-arm64", "linux-amd64", "linux-arm64"]
        .into_iter()
        .map(|target| (target, "tar.gz", "0".repeat(64)))
        .chain([
            ("windows-amd64", "zip", format!("{:x}", Sha256::digest(zip))),
            ("windows-arm64", "zip", "0".repeat(64)),
        ])
        .map(|(target, extension, sha256)| {
            let (os, arch) = target.split_once('-').unwrap();
            json!({"filename": format!("go1.22.12.{target}.{extension}"), "os": os,
                "arch": arch, "sha256": sha256, "version": "go1.22.12", "kind": "archive"})
        })
        .collect();
    let index = json!([{"version": "go1.22.12", "stable": true, "files": files}]);
    let routes = [
        (INDEX_PATH.into(), index.to_string().into_bytes()),
        (ZIP.into(), zip.to_vec()),
    ];
    LoopbackServer::new(routes, |_| {}).unwrap()
}

fn go_only() -> Fixture {
    let f = Fixture::new(false);
    let root = f.owned.root();
    fs::remove_file(root.join("package.json")).unwrap();
    fs::remove_file(root.join(".nvmrc")).unwrap();
    fs::write(root.join("turbo.json"), GO).unwrap();
    fs::write(root.join("go.work"), "go 1.22\n").unwrap();
    f
}

fn installed(f: &Fixture) -> Current {
    Installed::open(&f.owned).current().unwrap().unwrap()
}

#[test]
fn go_zip_installs_exe_exports_locks_every_target_and_repeats_offline() {
    if unqualified_host() {
        return;
    }
    let f = go_only();
    let u = go_upstream(&go_zip());
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    assert_eq!(u.requests(), [INDEX_PATH, ZIP]);
    let lock: Value =
        serde_json::from_slice(&fs::read(f.owned.root().join("turbo.lock")).unwrap()).unwrap();
    let artifacts = &lock["tools"]["go"]["installation"]["artifacts"];
    let windows = &artifacts["windows-x64"]["distribution"];
    assert_eq!(windows["url"], format!("https://dl.google.com{ZIP}"));
    assert_eq!(windows["format"], "zip");
    assert_eq!(
        windows["executables"],
        json!({"go": "bin/go.exe", "gofmt": "bin/gofmt.exe"})
    );
    assert!(artifacts["linux-x64-gnu"].is_object() && artifacts["windows-arm64"].is_object());

    let current = installed(&f);
    let mut names: Vec<_> = fs::read_dir(&current.bin)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["go.exe", "gofmt.exe"]);
    let tree = current.tree("go").to_owned();
    for name in ["go.exe", "gofmt.exe"] {
        let target = tree.join("bin").join(name);
        assert!(same_file::is_same_file(current.bin.join(name), target).unwrap());
    }
    assert!(tree.join("src/go.mod").is_file() && !tree.join("test").exists());

    // Frozen, local and offline check repeats neither download nor republish.
    let manifest = f.manifest();
    let offline = LoopbackServer::new([], |_| {}).unwrap();
    for flags in [FROZEN, LOCAL, CHECK] {
        assert_eq!(f.run(&offline, flags).unwrap(), 0, "{flags:?}");
    }
    assert_eq!(offline.hits(), 0);
    assert_eq!(f.manifest(), manifest);
    f.no_execution();
}

/// The overrides managed `turbo run`, `turbo exec` and turbo's own Go probes
/// receive after revalidation.
fn run_overrides(f: &Fixture) -> Result<turborepo_run::EnvironmentVariableMap, String> {
    use turborepo_run::ManagedEnvironment;

    use crate::commands::managed::{ManagedTools, context};
    let root = f.owned.root();
    let config = root.join("turbo.json");
    ManagedTools::inspect(root, &config, true, Some(f.owned.home()), context)?
        .ok_or("unmanaged")?
        .revalidate()
}

#[test]
fn managed_run_and_exec_pin_a_plain_goroot_and_the_local_toolchain() {
    if unqualified_host() {
        return;
    }
    let f = go_only();
    let u = go_upstream(&go_zip());
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    let env = run_overrides(&f).unwrap();
    assert_eq!(env["GOTOOLCHAIN"], "local");
    let path = std::ffi::OsString::from(&env["PATH"]);
    let bin = std::env::split_paths(&path).next().unwrap();
    // The hard-linked go.exe finds its tree only through GOROOT, which keeps
    // the ordinary spelling Go prints and compares, like the PATH entry.
    let goroot = PathBuf::from(&env["GOROOT"]);
    for spelling in [&goroot, &bin] {
        assert!(
            !spelling.to_string_lossy().starts_with(r"\\?\"),
            "{spelling:?}"
        );
    }
    assert!(same_file::is_same_file(goroot.join("bin/go.exe"), bin.join("go.exe")).unwrap());
    assert!(goroot.join("src/go.mod").is_file());
    // PATHEXT resolves the bare `go` that tasks and probes spawn to the export.
    let go = which::which_in("go", Some(&path), f.owned.root()).unwrap();
    assert!(same_file::is_same_file(go, bin.join("go.exe")).unwrap());
    // cmd.exe stands in for go.exe; the line exits 7 only with both variables.
    // `raw_arg` keeps the quotes cmd.exe parses itself.
    use std::os::windows::process::CommandExt;
    let check =
        r#"/d /c go /d /c if "%GOTOOLCHAIN%"=="local" if exist "%GOROOT%\src\go.mod" exit 7"#;
    let status = Command::new(system32().join("cmd.exe"))
        .raw_arg(check)
        .envs(env.iter())
        .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(7));

    // Without the workspace flag, go.work is unread and the selection drifts.
    fs::write(f.owned.root().join("turbo.json"), ENABLED).unwrap();
    let error = run_overrides(&f).unwrap_err();
    assert!(error.contains("no longer match turbo.lock"), "{error}");
}

#[test]
fn powershell_hook_activates_go_like_managed_run_and_restores_on_leaving() {
    use crate::commands::hook::powershell_for_tests as hook;
    if unqualified_host() {
        return;
    }
    let f = go_only();
    let u = go_upstream(&go_zip());
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    let env = run_overrides(&f).unwrap();
    let bin = std::env::split_paths(&env["PATH"]).next().unwrap();
    let inside = format!("{};C:\\Windows", bin.display());
    let (web, home) = (f.owned.root().join("apps/web/src"), f.owned.home());
    // This shell points GOROOT at another Go and has no GOTOOLCHAIN.
    let shell = |name: &str| (name == "GOROOT").then(|| r"C:\Go".to_owned());
    let (entered, state) = hook(None, r"C:\Windows", &web, home, shell);
    let state = state.unwrap();
    assert_eq!(
        entered,
        format!(
            "$env:PATH = '{inside}'\n$env:GOROOT = '{}'\n$env:GOTOOLCHAIN = \
             'local'\n$env:__TURBO_HOOK_STATE = '{state}'\n",
            env["GOROOT"]
        )
    );
    // The next prompt changes nothing; leaving restores both exactly.
    let active = |name: &str| env.get(name).cloned();
    assert_eq!(
        hook(Some(state.as_str()), &inside, &web, home, active),
        (String::new(), None)
    );
    let (left, cleared) = hook(Some(state.as_str()), &inside, home, home, active);
    assert_eq!(
        left,
        "$env:PATH = 'C:\\Windows'\n$env:GOROOT = 'C:\\Go'\nRemove-Item -LiteralPath \
         Env:GOTOOLCHAIN -ErrorAction SilentlyContinue\nRemove-Item -LiteralPath \
         Env:__TURBO_HOOK_STATE -ErrorAction SilentlyContinue\n"
    );
    assert_eq!(cleared, None);
}

#[test]
fn damaged_go_exports_fail_check_and_frozen_setup_repairs_them() {
    if unqualified_host() {
        return;
    }
    let f = go_only();
    let u = go_upstream(&go_zip());
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    let manifest = f.manifest();
    fs::remove_file(installed(&f).bin.join("gofmt.exe")).unwrap();
    assert!(f.run(&u, CHECK).is_err());
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(u.requests(), [INDEX_PATH, ZIP, ZIP]);
    assert_ne!(f.manifest(), manifest);
    assert_eq!(f.run(&u, CHECK).unwrap(), 0);
    assert!(installed(&f).bin.join("gofmt.exe").is_file());
}
