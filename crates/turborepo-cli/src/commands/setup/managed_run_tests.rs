use std::{path::Path, sync::Arc};

use turborepo_run::{ManagedEnvironment, builder::RunBuilder};
use turborepo_signals::SignalHandler;
use turborepo_telemetry::events::command::CommandEventBuilder;

use super::*;
use crate::commands::{
    CommandBase,
    run::managed::{self, ManagedTools},
};

fn select(f: &Fixture) -> Result<Option<Arc<ManagedTools>>, String> {
    ManagedTools::inspect(
        f.owned.root(),
        &f.owned.root().join("turbo.json"),
        true,
        f.owned.home(),
        managed::context,
    )
}

const NODE_RUNTIME: &str = "#!/bin/sh\nprintf '%s\\n' \"$0\" \"$PATH\" \"$TURBO_HASH\" \"$*\" \
                            \"${RUSTUP_TOOLCHAIN-unset}\" > managed-result\n";

fn runnable() -> Fixture {
    let mut f = Fixture::new();
    f.declare(json!({"name":"repo","private":true,"packageManager":"pnpm@10.0.0"}));
    fs::write(
        f.owned.root().join("pnpm-workspace.yaml"),
        "packages:\n  - apps/*\n",
    )
    .unwrap();
    fs::write(
        f.owned.root().join("apps/web/package.json"),
        r#"{"name":"web","scripts":{"build":"node -e 'throw new Error(\"not host node\")'"}}"#,
    )
    .unwrap();
    fs::write(
        f.owned.root().join("pnpm-lock.yaml"),
        "lockfileVersion: '9.0'\nimporters:\n  .: {}\n  apps/web: {}\n",
    )
    .unwrap();
    fs::write(f.owned.root().join("turbo.json"),
        r#"{"futureFlags":{"experimentalSetup":true},"tasks":{"build":{"cache":false}},"noUpdateNotifier":true}"#).unwrap();
    // Managed Node trap doubles as a harmless fixture runtime. pnpm's shim must
    // dispatch through THIS executable; no host language runtime is required.
    let prefix = f.lock["tools"]["node"]["installation"]["artifacts"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()["distribution"]["rootPrefix"]
        .as_str()
        .unwrap();
    f.node = archive(prefix, &[("bin/node", NODE_RUNTIME)]);
    let distribution = f.lock["tools"]["node"]["installation"]["artifacts"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap();
    distribution["distribution"]["sha256"] = json!(format!("{:x}", Sha256::digest(&f.node)));
    f.save();
    f
}

fn input(f: &Fixture, mode: &str) -> turborepo_run::RunBuilderInput {
    let args = Args::parse_args(vec![
        "turbo".into(),
        "run".into(),
        "build".into(),
        "--cwd".into(),
        f.owned.root().into(),
        format!("--env-mode={mode}").into(),
        "--ui=stream".into(),
        "--cache=local:rw".into(),
    ])
    .unwrap();
    let root = turbopath::AbsoluteSystemPathBuf::try_from(f.owned.root()).unwrap();
    CommandBase::new(
        args,
        root,
        "managed-fixture",
        turborepo_ui::ColorConfig::new(true),
    )
    .unwrap()
    .run_builder_input()
    .unwrap()
}

fn execute(f: &Fixture, mode: &str, tools: Arc<ManagedTools>) -> String {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let handler = SignalHandler::new(futures::stream::pending());
        let (run, analytics) = RunBuilder::new(input(f, mode), None)
            .unwrap()
            .with_managed_environment(tools)
            .build(&handler, CommandEventBuilder::new("run"))
            .await
            .unwrap();
        assert!(analytics.is_none());
        let result = run.run(None, false).await;
        handler.close().await;
        assert_eq!(result.unwrap(), 0);
    });
    fs::read_to_string(f.owned.root().join("apps/web/managed-result")).unwrap()
}

#[test]
fn setup_then_run_uses_managed_node_pnpm_in_strict_and_loose_modes() {
    for mode in ["strict", "loose"] {
        let f = runnable();
        let u = Upstream::new(&f, false, None);
        f.run(&u, FROZEN).unwrap();
        let tools = select(&f).unwrap().unwrap();
        let overrides = tools.revalidate().unwrap();
        let first = std::env::split_paths(std::ffi::OsStr::new(&overrides["PATH"]))
            .next()
            .unwrap();
        assert!(first.starts_with(f.owned.root()));
        let output = execute(&f, mode, tools);
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(
            Path::new(lines[0]).canonicalize().unwrap(),
            first.join("node").canonicalize().unwrap()
        );
        assert_eq!(
            std::env::split_paths(std::ffi::OsStr::new(lines[1]))
                .next()
                .unwrap(),
            first
        );
        assert!(!lines[2].is_empty());
        assert!(lines[3].contains("pnpm.cjs run build"));
        if mode == "strict" {
            assert_eq!(lines[4], "unset");
        } else {
            assert_eq!(
                lines[4],
                std::env::var("RUSTUP_TOOLCHAIN").unwrap_or("unset".into())
            );
        }
        assert_eq!(u.hits(), 3);
    }
}

#[test]
fn actual_task_hashes_are_location_independent_and_bind_verified_artifact_identity() {
    let mut hashes = vec![];
    let mut identities = vec![];
    for variant in [false, false, true] {
        let mut f = runnable();
        if variant {
            let distribution = f.lock["tools"]["node"]["installation"]["artifacts"]
                .as_object_mut()
                .unwrap()
                .values_mut()
                .next()
                .unwrap();
            let prefix = distribution["distribution"]["rootPrefix"].as_str().unwrap();
            f.node = archive(
                prefix,
                &[
                    ("bin/node", NODE_RUNTIME),
                    ("resource", "different verified bytes"),
                ],
            );
            distribution["distribution"]["sha256"] =
                json!(format!("{:x}", Sha256::digest(&f.node)));
            f.save();
        }
        let u = Upstream::new(&f, false, None);
        f.run(&u, FROZEN).unwrap();
        let tools = select(&f).unwrap().unwrap();
        identities.push(tools.cache_identity().to_owned());
        hashes.push(
            execute(&f, "strict", tools)
                .lines()
                .nth(2)
                .unwrap()
                .to_owned(),
        );
        assert_eq!(u.hits(), 3);
    }
    assert_eq!(identities[0], identities[1]);
    assert_eq!(hashes[0], hashes[1]);
    assert_ne!(identities[0], identities[2]);
    assert_ne!(hashes[0], hashes[2]);
}

#[test]
fn declaration_drift_after_selection_fails_before_builder_work_or_task_dispatch() {
    let f = runnable();
    let u = Upstream::new(&f, false, None);
    f.run(&u, FROZEN).unwrap();
    let tools = select(&f).unwrap().unwrap();
    fs::write(f.owned.root().join(".nvmrc"), "26.0.0").unwrap();
    let error = tokio::runtime::Runtime::new().unwrap().block_on(async {
        let handler = SignalHandler::new(futures::stream::pending());
        let result = RunBuilder::new(input(&f, "strict"), None)
            .unwrap()
            .with_managed_environment(tools)
            .build(&handler, CommandEventBuilder::new("run"))
            .await;
        handler.close().await;
        match result {
            Err(error) => error.to_string(),
            Ok(_) => panic!("stale run succeeded"),
        }
    });
    assert!(error.contains("run turbo setup locally"));
    assert!(error.contains("commit the changed turbo.lock"));
    assert!(!f.owned.root().join("apps/web/managed-result").exists());
    assert_eq!(u.hits(), 3);
}

#[test]
fn flag_and_exact_lock_root_gate_selection_without_ancestor_fallback_or_host_qualification() {
    let f = Fixture::new();
    let config = f.owned.root().join("turbo.json");
    assert!(
        ManagedTools::inspect(f.owned.root(), &config, false, f.owned.home(), || panic!(
            "disabled repo probed host"
        ))
        .unwrap()
        .is_none()
    );
    let nested = f.owned.root().join("apps/web");
    assert!(
        ManagedTools::inspect(
            &nested,
            &nested.join("turbo.json"),
            true,
            f.owned.home(),
            || panic!("missing exact-root lock probed host")
        )
        .unwrap()
        .is_none()
    );
    assert!(select(&f).is_err()); // Lock exists, but setup never prepared evidence.
    assert!(!f.owned.home().join("turborepo").exists());
    assert!(!f.owned.root().join(".turbo").exists());
    fs::write(
        f.owned.root().join("Cargo.toml"),
        "[workspace]\nmembers = []\n",
    )
    .unwrap();
    let mut opts = input(&f, "strict").opts;
    opts.future_flags.experimental_cargo_workspaces = true;
    let root = turbopath::AbsoluteSystemPathBuf::try_from(f.owned.root()).unwrap();
    let base = CommandBase::from_opts(opts, root, "fixture", turborepo_ui::ColorConfig::new(true));
    assert!(
        managed::select(&base)
            .err()
            .unwrap()
            .to_string()
            .contains("Node/pnpm JS")
    );
}

#[test]
fn managed_overrides_do_not_modify_host_environment_and_damage_never_falls_back() {
    let f = runnable();
    let u = Upstream::new(&f, false, None);
    f.run(&u, FROZEN).unwrap();
    let before: std::collections::BTreeMap<_, _> = std::env::vars_os().collect();
    let tools = select(&f).unwrap().unwrap();
    let environment = tools.revalidate().unwrap();
    assert_eq!(environment.len(), 1); // PATH only; no unrelated host env exposed.
    let current = turborepo_tool_install::Store::inspect(f.owned.root())
        .unwrap()
        .unwrap();
    fs::remove_file(current.bin.join("pnpm")).unwrap();
    assert!(tools.revalidate().is_err());
    assert!(select(&f).is_err());
    assert_eq!(
        std::env::vars_os().collect::<std::collections::BTreeMap<_, _>>(),
        before
    );
    assert_eq!(u.hits(), 3);
    f.run(&u, FROZEN).unwrap(); // Explicit setup repairs the complete cohort.
    assert!(!execute(&f, "strict", select(&f).unwrap().unwrap()).is_empty());
    assert_eq!(u.hits(), 6);
}
