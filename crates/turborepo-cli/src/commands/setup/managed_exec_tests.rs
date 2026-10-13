use std::{ffi::OsString, path::Path};

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
        f.owned.home(),
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
    let current = turborepo_tool_install::Store::inspect(root)
        .unwrap()
        .unwrap();
    fs::remove_file(current.bin.join("pnpm")).unwrap();
    assert!(matches!(
        exec::spawn(tools.as_ref(), &command, &cwd, launch),
        Err(exec::Error::Stale(_))
    ));
    assert!(!result.exists());
    // Exec never downloads or repairs; only setup contacted the upstream.
    assert_eq!(u.hits(), 3);
}
