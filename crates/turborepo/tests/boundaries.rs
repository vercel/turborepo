mod common;

#[test]
fn root_inspection_and_printing_scripts_are_not_recursive() -> Result<(), anyhow::Error> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    std::fs::write(
        root.join("package.json"),
        r#"{
        "name": "root-recursion-repro", "private": true,
        "packageManager": "npm@10.5.0", "workspaces": ["packages/*"],
        "scripts": {
            "boundaries": "turbo boundaries",
            "echo-turbo": "echo turbo",
            "printf-turbo": "printf turbo",
            "query": "turbo query \"{ packages { items { name } } }\"",
            "ls": "turbo ls --output=json",
            "info": "turbo info",
            "bin": "turbo bin",
            "help": "turbo --help",
            "short-help": "turbo -h",
            "version": "turbo --version",
            "recursive": "turbo run recursive --dry=json",
            "wrapper-script": "pnpm x turbo boundaries",
            "x": "node -e \"require('fs').writeFileSync('wrapper-dispatched', 'yes')\" && turbo run recursive --dry=json"
        }
    }"#,
    )?;
    std::fs::write(
        root.join("package-lock.json"),
        r#"{
        "name": "root-recursion-repro", "lockfileVersion": 3,
        "packages": {
            "": {"name": "root-recursion-repro", "workspaces": ["packages/*"]},
            "packages/demo": {"name": "demo", "version": "1.0.0"},
            "node_modules/demo": {"resolved": "packages/demo", "link": true}
        }
    }"#,
    )?;
    std::fs::create_dir_all(root.join("packages/demo"))?;
    std::fs::write(
        root.join("packages/demo/package.json"),
        r#"{"name": "demo", "version": "1.0.0", "private": true}"#,
    )?;
    let bin_dir = std::path::Path::new(env!("CARGO_BIN_EXE_turbo"))
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Turbo binary has no parent directory"))?;
    let path = common::setup::prepend_to_path(bin_dir);
    // Each check spawns turbo -> npm -> turbo, which is slow on Windows. Run
    // them as one parallel invocation; the bound only catches real hangs.
    let timeout = std::time::Duration::from_secs(if cfg!(windows) { 120 } else { 60 });
    let version = turborepo_cli::get_version();
    let mut checks = vec![
        ("boundaries", "no issues found"),
        ("echo-turbo", "turbo"),
        ("query", "\"demo\""),
        ("ls", "\"demo\""),
        ("info", "Package manager: npm"),
        ("bin", env!("CARGO_BIN_EXE_turbo")),
        ("help", "Usage: turbo"),
        ("short-help", "Usage: turbo"),
        ("version", version),
    ];
    if !cfg!(windows) {
        checks.push(("printf-turbo", "turbo"));
    }
    let tasks: std::collections::BTreeMap<_, _> = checks
        .iter()
        .map(|(task, _)| (format!("//#{}", task), serde_json::json!({"cache": false})))
        .chain(
            ["recursive", "wrapper-script"]
                .into_iter()
                .map(|task| (format!("//#{}", task), serde_json::json!({"cache": false}))),
        )
        .collect();
    std::fs::write(
        root.join("turbo.json"),
        serde_json::to_vec(&serde_json::json!({"tasks": tasks}))?,
    )?;
    let output = common::turbo_command(root)
        .env("PATH", &path)
        .arg("run")
        .args(checks.iter().map(|(task, _)| *task))
        .args(["--filter=//", "--skip-infer", "--continue"])
        .timeout(timeout)
        .output()?;
    let combined = common::combined_output(&output);
    assert!(
        output.status.success(),
        "exited with {:?}: {combined}",
        output.status
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for (task, expected) in checks {
        // Skip npm's `> script` echo lines so only the script's output counts.
        let prefix = format!("//:{task}: ");
        let ran = stdout
            .lines()
            .filter_map(|line| line.strip_prefix(&prefix))
            .filter(|line| !line.starts_with("> "))
            .any(|line| line.contains(expected));
        assert!(ran, "{task} did not print {expected:?}: {combined}");
    }
    // The child would only produce a dry run if accidentally dispatched, so a
    // broken guard cannot start an unbounded recursive process tree in CI.
    for task in ["recursive", "wrapper-script"] {
        let output = common::turbo_command(root)
            .env("PATH", &path)
            .args(["run", task, "--filter=//", "--skip-infer"])
            .timeout(timeout)
            .output()?;
        let combined = common::combined_output(&output);
        assert!(
            !output.status.success(),
            "{task} exited with {:?}: {combined}",
            output.status
        );
        assert!(
            combined.contains("recursive_turbo_invocations"),
            "{combined}"
        );
        assert!(
            !root.join("wrapper-dispatched").exists(),
            "a package script was dispatched before the recursion guard: {combined}"
        );
    }
    Ok(())
}

#[test]
fn test_boundaries() -> Result<(), anyhow::Error> {
    check_json_output!(
        "boundaries",
        "npm@10.5.0",
        "query",
        "get boundaries lints" => ["query { boundaries { items { message import } } }"],
    );

    Ok(())
}

#[test]
fn test_boundaries_tags() -> Result<(), anyhow::Error> {
    check_json_output!(
        "boundaries_tags",
        "npm@10.5.0",
        "query",
        "get boundaries lints" => ["query { boundaries { items { message import } } }"],
    );

    Ok(())
}

#[test]
fn test_boundaries_on_basic_monorepo() -> Result<(), anyhow::Error> {
    check_json_output!(
        "basic_monorepo",
        "npm@10.5.0",
        "query",
        "get boundaries lints" => ["query { boundaries { items { message import } } }"],
    );

    Ok(())
}

#[test]
fn test_boundaries_circular() -> Result<(), anyhow::Error> {
    check_json_output!(
        "boundaries_circular",
        "npm@10.5.0",
        "query",
        "get boundaries lints" => ["query { boundaries { items { message import } } }"],
    );

    Ok(())
}
