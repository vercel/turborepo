use std::{collections::BTreeMap, ffi::OsString, path::Path};

use turbopath::AbsoluteSystemPathBuf;

use super::*;
use crate::commands::{
    exec,
    managed::{self, ManagedTools},
};

const NODE_RUNTIME: &str =
    "#!/bin/sh\npwd > exec-result\nprintf '%s\\n' \"$0\" \"$@\" >> exec-result\nexit 3\n";

fn launch(mut command: std::process::Command) -> std::io::Result<std::process::ExitStatus> {
    command.status()
}

#[test]
fn exec_runs_authorized_managed_tools_and_never_runs_stale_ones() {
    let mut f = Fixture::new();
    let distribution = f.lock["tools"]["node"]["installation"]["artifacts"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap();
    let prefix = distribution["distribution"]["rootPrefix"]
        .as_str()
        .unwrap()
        .to_owned();
    f.node = archive(&prefix, &[("bin/node", NODE_RUNTIME)]);
    distribution["distribution"]["sha256"] = json!(format!("{:x}", Sha256::digest(&f.node)));
    f.save();
    let u = Upstream::new(&f, false, None);
    f.run(&u, FROZEN).unwrap();

    let root = f.owned.root();
    let tools = ManagedTools::inspect(
        root,
        &root.join("turbo.json"),
        true,
        Some(f.owned.home()),
        managed::context,
    )
    .unwrap()
    .unwrap();
    let managed_bin = std::env::split_paths(&tools.revalidate().unwrap()["PATH"])
        .next()
        .unwrap();
    let cwd = AbsoluteSystemPathBuf::try_from(root.join("apps/web")).unwrap();
    let command = ["node", "--version", "a b"].map(OsString::from);
    assert_eq!(
        exec::spawn(tools.as_ref(), &command, &cwd, launch).unwrap(),
        3
    );
    let result = root.join("apps/web/exec-result");
    let record = fs::read_to_string(&result).unwrap();
    let lines: Vec<_> = record.lines().collect();
    assert_eq!(
        Path::new(lines[0]).canonicalize().unwrap(),
        cwd.as_std_path().canonicalize().unwrap()
    );
    assert_eq!(
        Path::new(lines[1]).canonicalize().unwrap(),
        managed_bin.join("node").canonicalize().unwrap()
    );
    assert_eq!(&lines[2..], ["--version", "a b"]);

    // A damaged cohort fails at the spawn boundary, never on host tools.
    fs::remove_file(&result).unwrap();
    let current = Installed::open(&f.owned).current().unwrap().unwrap();
    fs::remove_file(current.bin.join("pnpm")).unwrap();
    assert!(matches!(
        exec::spawn(tools.as_ref(), &command, &cwd, launch),
        Err(exec::Error::Stale(_))
    ));
    assert!(!result.exists());
    // Exec never downloads or repairs; only setup contacted the upstream.
    assert_eq!(u.hits(), 3);
}

fn installed_tools(f: &Fixture) -> std::sync::Arc<ManagedTools> {
    let root = f.owned.root();
    ManagedTools::inspect_tools(
        root,
        &root.join("turbo.json"),
        true,
        Some(f.owned.home()),
        managed::context,
    )
    .unwrap()
    .unwrap()
}

/// The environment a program `turbo exec` ran actually saw.
fn child_env(tools: &ManagedTools, program: &[&str], cwd: &Path) -> BTreeMap<String, String> {
    let output = cwd.join("child-env");
    let command: Vec<_> = program.iter().map(OsString::from).collect();
    let cwd = AbsoluteSystemPathBuf::try_from(cwd).unwrap();
    let status = exec::spawn(tools, &command, &cwd, |mut command| {
        command.stdout(fs::File::create(&output)?);
        command.status()
    })
    .unwrap();
    assert_eq!(status, 0);
    fs::read_to_string(&output)
        .unwrap()
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

#[test]
fn print_env_is_the_environment_exec_gives_the_program() {
    use crate::cli::PrintEnv;
    let f = Fixture::new();
    f.run(&Upstream::new(&f, false, None), FROZEN).unwrap();
    let tools = installed_tools(&f);
    let (overrides, bin) = tools.activation().unwrap();
    let json = exec::render(PrintEnv::Json, &overrides, &bin).unwrap();
    let printed: BTreeMap<String, String> = serde_json::from_str(&json).unwrap();
    assert!(printed.contains_key("PATH"));
    let cwd = f.owned.root().join("apps/web");
    let seen = child_env(&tools, &["env"], &cwd);
    for (key, value) in &printed {
        assert_eq!(seen.get(key), Some(value), "{key}");
    }
    // A shell that evaluates `sh` output gets the same values.
    let script = exec::render(PrintEnv::Sh, &overrides, &bin).unwrap();
    fs::write(cwd.join("activate.sh"), &script).unwrap();
    let evaluated = child_env(&tools, &["sh", "-c", ". ./activate.sh && env"], &cwd);
    for (key, value) in &printed {
        assert_eq!(evaluated.get(key), Some(value), "{key}");
    }
    // `bin` is the repository's stable link to the same environment.
    let bin = exec::render(PrintEnv::Bin, &overrides, &bin).unwrap();
    let bin = Path::new(bin.trim_end());
    assert!(bin.starts_with(f.owned.home()) && bin.ends_with("bin"));
    let first = std::env::split_paths(&printed["PATH"]).next().unwrap();
    assert_eq!(bin.canonicalize().unwrap(), first.canonicalize().unwrap());
    f.no_execution();
}

#[test]
fn exec_after_lock_drift_fails_with_setup_guidance_and_runs_nothing() {
    for case in 0..2 {
        let mut f = Fixture::new();
        f.run(&Upstream::new(&f, false, None), FROZEN).unwrap();
        let tools = installed_tools(&f);
        match case {
            // Other bytes for the same tools: the record no longer matches.
            0 => fs::write(f.owned.root().join("turbo.lock"), format!("{}\n", f.lock)).unwrap(),
            // Another resolution than the declarations ask for.
            _ => {
                f.lock["tools"]["node"]["version"] = json!("24.0.1");
                f.save();
            }
        }
        let cwd = AbsoluteSystemPathBuf::try_from(f.owned.root().join("apps/web")).unwrap();
        let command = ["node", "--version"].map(OsString::from);
        let error = exec::spawn(tools.as_ref(), &command, &cwd, |_| {
            panic!("a stale selection must never run")
        })
        .unwrap_err();
        assert!(matches!(error, exec::Error::Stale(_)), "{error}");
        assert!(
            error.to_string().contains("turbo setup"),
            "case {case}: {error}"
        );
        let root = f.owned.root();
        let fresh = ManagedTools::inspect_tools(
            root,
            &root.join("turbo.json"),
            true,
            Some(f.owned.home()),
            managed::context,
        );
        assert!(fresh.err().unwrap().contains("turbo setup"), "case {case}");
        f.no_execution();
    }
}
