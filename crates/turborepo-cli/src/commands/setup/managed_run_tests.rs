use std::{path::Path, sync::Arc};

use turborepo_run::{ManagedEnvironment, ManagedRun, builder::RunBuilder};
use turborepo_signals::SignalHandler;
use turborepo_telemetry::events::command::CommandEventBuilder;

use super::*;
use crate::commands::{
    CommandBase,
    managed::{self, ManagedTools},
};

fn select(f: &Fixture) -> Result<Option<Arc<ManagedRun>>, String> {
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
    let mut input = CommandBase::new(
        args,
        root,
        "managed-fixture",
        turborepo_ui::ColorConfig::new(true),
    )
    .unwrap()
    .run_builder_input()
    .unwrap();
    // CI exports Remote Cache credentials; these fixtures must stay offline.
    input.api_auth = None;
    input
}

fn execute(f: &Fixture, mode: &str, tools: Arc<ManagedRun>) -> String {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let handler = SignalHandler::new(futures::stream::pending());
        let mut input = input(f, mode);
        input.managed_environment = Some(tools);
        let (run, analytics) = RunBuilder::new(input, None)
            .unwrap()
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
fn concrete_carrier_rejects_an_environment_for_a_different_snapshot() {
    struct Wrong;
    impl ManagedEnvironment for Wrong {
        fn revalidate(&self) -> Result<turborepo_run::EnvironmentVariableMap, String> {
            panic!("mismatched carrier must not use the environment")
        }
        fn cache_identity(&self) -> &str {
            "wrong"
        }
    }
    let f = runnable();
    let u = Upstream::new(&f, false, None);
    f.run(&u, FROZEN).unwrap();
    let tools = select(&f).unwrap().unwrap();
    assert!(ManagedRun::new(tools.snapshot().clone(), Arc::new(Wrong)).is_err());
    assert_eq!(u.hits(), 3);
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
        let mut input = input(&f, "strict");
        input.managed_environment = Some(tools);
        let result = RunBuilder::new(input, None)
            .unwrap()
            .build(&handler, CommandEventBuilder::new("run"))
            .await;
        handler.close().await;
        match result {
            Err(error) => error.to_string(),
            Ok(_) => panic!("stale run succeeded"),
        }
    });
    assert!(error.contains("changed during this run"), "{error}");
    assert!(error.contains("after turbo setup if declarations changed"));
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
    // Cargo repositories are no longer refused; they need the same setup.
    fs::write(
        f.owned.root().join("Cargo.toml"),
        "[workspace]\nmembers = []\n",
    )
    .unwrap();
    let cargo = r#"{"futureFlags":{"experimentalSetup":true,"experimentalCargoWorkspaces":true}}"#;
    fs::write(&config, cargo).unwrap();
    let error = select(&f).err().unwrap();
    assert!(error.contains("run turbo setup"), "{error}");
    assert!(!f.owned.root().join(".turbo").exists());
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

#[test]
fn guidance_says_whether_setup_repairs_tools_or_updates_the_lock() {
    use turborepo_setup::activation::Error as Plan;

    use crate::commands::setup::authorization::Error;
    let repair = "; run turbo setup to repair the installation (turbo.lock is unchanged)";
    for ci in [false, true] {
        let drift = Error::Plan(Plan::DeclarationDrift).guidance(ci);
        assert!(drift.starts_with("tool declarations no longer match turbo.lock; "));
        // CI setup is frozen, so CI is sent back to a local lock update.
        assert_eq!(
            drift.contains("run turbo setup locally and commit the updated turbo.lock"),
            ci
        );
        assert_eq!(
            drift.contains("run turbo setup to update turbo.lock, then commit it"),
            !ci
        );
        for error in [
            Error::Plan(Plan::MissingInventory),
            Error::Plan(Plan::StaleInventory),
            Error::Untrusted,
            Error::Changed,
        ] {
            let text = error.guidance(ci);
            assert!(text.ends_with(repair) && !text.contains("commit"), "{text}");
        }
        // Setup rejects an invalid lock too, so it is never the whole answer.
        let invalid = Error::Plan(Plan::InvalidLock).guidance(ci);
        assert!(
            invalid.contains("restore the committed turbo.lock, or delete it and run turbo setup"),
            "{invalid}"
        );
        // Unsupported inputs keep their own next action.
        assert_eq!(
            Error::Plan(Plan::UnsupportedAdapter).guidance(ci),
            Plan::UnsupportedAdapter.to_string()
        );
    }
}

#[test]
fn deleted_shims_and_declaration_drift_never_download_and_setup_recovers() {
    use turborepo_setup::activation::Error as Plan;

    use crate::commands::setup::authorization::Error;
    let f = runnable();
    let u = Upstream::new(&f, false, None);
    f.run(&u, FROZEN).unwrap();
    let lock = fs::read(f.owned.root().join("turbo.lock")).unwrap();
    let current = turborepo_tool_install::Store::inspect(f.owned.root())
        .unwrap()
        .unwrap();
    fs::remove_file(current.bin.join("node")).unwrap();
    let hits = u.hits();
    let error = select(&f).err().unwrap();
    assert!(
        error.contains("run turbo setup to repair the installation"),
        "{error}"
    );
    assert_eq!(u.hits(), hits); // Run never downloads.
    // The recommended setup also works in CI: frozen setup repairs from the lock.
    f.run(&u, FROZEN).unwrap();
    assert!(select(&f).unwrap().is_some());
    assert_eq!(fs::read(f.owned.root().join("turbo.lock")).unwrap(), lock);

    fs::write(f.owned.root().join(".nvmrc"), "24.x\n").unwrap();
    let hits = u.hits();
    assert_eq!(
        select(&f).err().unwrap(),
        Error::Plan(Plan::DeclarationDrift).guidance(turborepo_ci::is_ci())
    );
    // Frozen setup refuses the stale lock, matching the CI guidance, while
    // local setup updates it so the next run is ready instead of looping.
    assert!(f.run(&u, FROZEN).is_err());
    assert_eq!(u.hits(), hits);
    // Only Node is re-resolved; pnpm keeps its locked bytes.
    let mut routes = super::first_lock::routes(&f, "24.0.0", "10.0.0");
    routes.retain(|(path, _)| path.starts_with("/dist/"));
    let integrity = super::first_lock::base64(&sha2::Sha512::digest(&f.pnpm));
    let metadata = json!({"name":"pnpm","version":"10.0.0","dist":{
        "tarball":"https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz",
        "integrity":format!("sha512-{integrity}")}});
    routes.push(("/pnpm/10.0.0".into(), metadata.to_string().into_bytes()));
    routes.push(("/pnpm/-/pnpm-10.0.0.tgz".into(), f.pnpm.clone()));
    let resolver = LoopbackServer::new(routes, |_| {}).unwrap();
    f.run(&resolver, &["--tools-only", "--no-frozen"]).unwrap();
    assert_ne!(fs::read(f.owned.root().join("turbo.lock")).unwrap(), lock);
    assert!(select(&f).unwrap().is_some());
}

const NO_LOCK: &[&str] = &["--no-lock", "--tools-only"];

/// `runnable()` provisioned by `turbo setup --no-lock`, so without turbo.lock.
fn clone_local() -> Fixture {
    let f = runnable();
    fs::remove_file(f.owned.root().join("turbo.lock")).unwrap();
    let routes = super::first_lock::routes_with(&f, "24.0.0", "10.0.0", NODE_RUNTIME);
    f.run(&LoopbackServer::new(routes, |_| {}).unwrap(), NO_LOCK)
        .unwrap();
    assert!(!f.owned.root().join("turbo.lock").exists());
    f
}

#[test]
fn no_lock_setup_then_run_uses_clone_local_tools_in_strict_and_loose_modes() {
    for mode in ["strict", "loose"] {
        let f = clone_local();
        let tools = select(&f).unwrap().unwrap();
        let identity: Value =
            serde_json::from_slice(tools.snapshot().canonical_identity()).unwrap();
        assert_eq!(identity["resolution"], "clone-local");
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
        assert!(lines[3].contains("pnpm.cjs run build"));
        f.no_execution();
    }
}

#[test]
fn no_lock_task_hashes_never_match_the_same_tools_under_a_committed_lock() {
    let f = clone_local();
    let local = select(&f).unwrap().unwrap();
    let identity = local.cache_identity().to_owned();
    let tools = local.snapshot().tools().clone();
    let hash = execute(&f, "strict", local)
        .lines()
        .nth(2)
        .unwrap()
        .to_owned();
    let current = turborepo_tool_install::Store::inspect(f.owned.root())
        .unwrap()
        .unwrap();
    let selection = current.record.unwrap();
    fs::write(f.owned.root().join("turbo.lock"), selection.bytes()).unwrap();
    // Setup from that lock records its digest for the same generation.
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    f.run(&empty, NO_LOCK).unwrap();
    assert_eq!(empty.hits(), 0);
    let locked = select(&f).unwrap().unwrap();
    assert_eq!(locked.snapshot().tools(), &tools);
    assert_ne!(locked.cache_identity(), identity);
    assert_ne!(execute(&f, "strict", locked).lines().nth(2).unwrap(), hash);
}

#[test]
fn no_lock_run_fails_closed_with_clone_local_guidance() {
    for case in 0..4 {
        let f = clone_local();
        let root = f.owned.root();
        let current = turborepo_tool_install::Store::inspect(root)
            .unwrap()
            .unwrap();
        match case {
            0 => fs::write(root.join(".nvmrc"), "24.1.0\n").unwrap(),
            1 => fs::remove_dir_all(current.bin.parent().unwrap()).unwrap(),
            2 => {
                for entry in fs::read_dir(f.owned.home().join("turborepo/setup-roots")).unwrap() {
                    fs::remove_file(entry.unwrap().path()).unwrap();
                }
            }
            // Removing the clone-local inventory is the opt-out.
            _ => fs::remove_dir_all(root.join(".turbo/tools")).unwrap(),
        }
        match select(&f) {
            Ok(None) if case == 3 => {}
            Err(error) if case < 3 => {
                assert!(
                    error.contains("run turbo setup --no-lock"),
                    "{case}: {error}"
                );
                assert!(!error.contains("commit"), "{error}");
            }
            other => panic!("case {case}: {:?}", other.map(|tools| tools.is_some())),
        }
        f.no_execution();
    }
}

#[path = "additional_activation_tests.rs"]
mod additional;
