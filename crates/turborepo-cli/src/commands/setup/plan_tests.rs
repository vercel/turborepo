use super::{checks::snapshot, *};

const PLAN: &[&str] = &["--plan", "--tools-only"];

impl Fixture {
    fn plan_report(&self, force: bool) -> Result<String, Error> {
        self.raw_plan(force).map(|report| trees(&report))
    }

    fn raw_plan(&self, force: bool) -> Result<String, Error> {
        let discovery = root::Discovery::capture(&self.args(PLAN), self.git.clone())?;
        let sources = self.sources()?;
        plan::inspect(&discovery, sources, force, self.owned.home(), || {
            Ok(self
                .owned
                .policy_at(&self.owned.root().join("apps/web/src"))?)
        })
    }

    fn expected_plan(&self, action: &str) -> String {
        let (node, pnpm) = match action {
            "reuse" => (
                "\n    Installed: <tree>/bin/node",
                "\n    Installed: <tree>",
            ),
            _ => ("", ""),
        };
        format!(
            "Using versions in turbo.lock:\n\nToolchains\n\
             \x20 node 24.0.0: {action}{node}\n    Declaration: .nvmrc (24.0.0)\n    Source: https://nodejs.org{}\n    SHA-256: {:x}\n\n\
             \x20 pnpm 10.0.0: {action}{pnpm}\n    Declaration: package.json#/packageManager (pnpm@10.0.0)\n    Source: https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz\n    SHA-256: {:x}",
            self.node_path,
            Sha256::digest(&self.node),
            Sha256::digest(&self.pnpm),
        )
    }

    fn assert_plan(&self, upstream: &Upstream, flags: &[&str], action: &str) {
        let before = snapshot(self.owned.root().parent().unwrap());
        let hits = upstream.hits();
        assert_eq!(self.run(upstream, flags).unwrap(), 0);
        assert_eq!(
            self.plan_report(flags.contains(&"--force")).unwrap(),
            self.expected_plan(action),
        );
        assert_eq!(snapshot(self.owned.root().parent().unwrap()), before);
        assert_eq!(upstream.hits(), hits);
        self.no_execution();
    }
}

#[test]
fn locked_plans_report_missing_reuse_and_force_in_every_readonly_mode() {
    let f = Fixture::new();
    let u = Upstream::new(&f, false, None);
    for installed in [false, true] {
        if installed {
            f.install(&u);
        }
        for extra in [
            None,
            Some("--frozen"),
            Some("--offline"),
            Some("--no-frozen"),
        ] {
            let mut flags = PLAN.to_vec();
            flags.extend(extra);
            f.assert_plan(
                &u,
                &flags,
                if installed {
                    "reuse"
                } else {
                    "install (missing installation)"
                },
            );
            flags.push("--force");
            f.assert_plan(&u, &flags, "re-verify (--force)");
        }
        assert_eq!(f.owned.root().join(".turbo").exists(), installed);
    }
    let report = f.raw_plan(false).unwrap();
    let current = Installed::open(&f.owned).current().unwrap().unwrap();
    let node = current.tree("node").join("bin/node");
    assert!(
        report.contains(&format!("\n    Installed: {}\n", node.display())),
        "{report}"
    );
    assert_eq!(u.hits(), 3);
    let sources = f.sources().unwrap();
    let writer = sources.guard().unwrap();
    let store = Installed::open(&f.owned);
    std::thread::scope(|scope| {
        let (send, receive) = std::sync::mpsc::channel();
        let fixture = &f;
        let pending = scope.spawn(move || send.send(fixture.plan_report(false)).unwrap());
        let result = receive.recv_timeout(std::time::Duration::from_secs(5));
        drop(store);
        drop(writer);
        pending.join().unwrap();
        assert_eq!(result.unwrap().unwrap(), f.expected_plan("reuse"));
    });
    assert_eq!(u.hits(), 3);
}

#[test]
fn node_only_plan_ignores_disabled_ecosystems_without_probing_or_creating_storage() {
    let mut f = Fixture::new();
    f.lock["tools"].as_object_mut().unwrap().remove("pnpm");
    f.declare(json!({}));
    for name in ["Cargo.toml", "pyproject.toml", "go.mod"] {
        fs::write(f.owned.root().join(name), "not enabled; must not probe").unwrap();
    }
    let u = Upstream::new(&f, false, None);
    let before = snapshot(f.owned.root().parent().unwrap());
    assert_eq!(f.run(&u, PLAN).unwrap(), 0);
    assert_eq!(f.plan_report(false).unwrap(), format!(
        "Using versions in turbo.lock:\n\nToolchains\n\
         \x20 node 24.0.0: install (missing installation)\n    Declaration: .nvmrc (24.0.0)\n    Source: https://nodejs.org{}\n    SHA-256: {:x}",
        f.node_path, Sha256::digest(&f.node),
    ));
    assert_eq!(snapshot(f.owned.root().parent().unwrap()), before);
    assert_eq!(u.hits(), 0);
    assert!(!f.owned.root().join(".turbo").exists());
}

#[test]
fn damaged_plans_never_change_any_owned_bytes() {
    for case in 0..5 {
        let mut f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        f.install(&u);
        let current = Installed::open(&f.owned).current().unwrap().unwrap();
        match case {
            0 => turborepo_setup::test_support::damage(
                &current.tree("node").join("resource"),
                "damage",
            ),
            1 => fs::remove_file(current.tree("pnpm").join("dist/resource")).unwrap(),
            2 => fs::remove_dir_all(current.bin.parent().unwrap()).unwrap(),
            3 => {
                turborepo_setup::test_support::damage(
                    &current.tree("node").join("resource"),
                    "damage",
                );
                fs::remove_file(current.bin.join("node")).unwrap();
                std::os::unix::fs::symlink(
                    current.tree("pnpm").join("bin/pnpm.cjs"),
                    current.bin.join("node"),
                )
                .unwrap();
            }
            _ => {
                f.lock["tools"]["node"]["installation"]["artifacts"]
                    .as_object_mut()
                    .unwrap()
                    .values_mut()
                    .next()
                    .unwrap()["distribution"]["sha256"] = json!("0".repeat(64));
                f.save();
            }
        }
        let before = snapshot(f.owned.root().parent().unwrap());
        for force in [false, true] {
            let mut flags = PLAN.to_vec();
            if force {
                flags.push("--force");
            }
            assert_eq!(f.run(&u, &flags).unwrap(), 0);
            // Like run and exec, a plan reads shims and seals, never tree
            // contents: damage inside a tree is for --check to find.
            let action = match case {
                _ if force => "re-verify (--force)",
                2 | 3 => "repair (damaged installation)",
                4 => "repair (installation does not match turbo.lock)",
                _ => "reuse",
            };
            let report = f.plan_report(force).unwrap();
            assert!(report.contains(action), "case {case}: {report}");
            if case != 4 {
                assert_eq!(report, f.expected_plan(action));
            }
        }
        assert_eq!(snapshot(f.owned.root().parent().unwrap()), before);
        assert_eq!(u.hits(), 3);
        if matches!(case, 0 | 1) {
            assert_eq!(
                f.run(&u, &["--force", "--tools-only", "--frozen"]).unwrap(),
                0
            );
            // Only the damaged tree is fetched again: Node's archive, or
            // pnpm's exact metadata and tarball.
            assert_eq!(u.hits(), if case == 0 { 4 } else { 5 });
            assert_eq!(f.plan_report(false).unwrap(), f.expected_plan("reuse"));
        }
        f.no_execution();
    }
}

#[test]
fn missing_stale_unsupported_and_denied_plans_fail_without_writes_or_traffic() {
    for case in 0..11 {
        let mut f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        let mut flags = PLAN.to_vec();
        match case {
            // Local plans propose these (below); frozen ones must fail.
            0 => {
                fs::remove_file(f.owned.root().join("turbo.lock")).unwrap();
                flags.push("--frozen");
            }
            1 => {
                fs::write(f.owned.root().join(".nvmrc"), "26.x").unwrap();
                flags.push("--frozen");
            }
            2 => fs::write(f.owned.root().join("turbo.lock"), "invalid").unwrap(),
            3 => {
                f.lock["tools"]["node"]["adapter"] = json!("generic-download");
                f.save();
            }
            4 => {
                f.lock["tools"]["pnpm"]["version"] = json!("11.0.0");
                f.save();
            }
            5 => fs::write(f.owned.root().join("turbo.json"), "{}").unwrap(),
            6 => fs::write(
                f.owned.root().join("turbo.json"),
                r#"{"futureFlags":{"experimentalSetup":true},"setup":{}}"#,
            )
            .unwrap(),
            7 => flags.push("--no-lock"),
            8 => flags.extend(["--update-lock", "--no-frozen", "--force"]),
            9 => {
                let artifacts = f.lock["tools"]["node"]["installation"]["artifacts"]
                    .as_object_mut()
                    .unwrap();
                let value = artifacts.values().next().unwrap().clone();
                artifacts.clear();
                artifacts.insert("windows-x64".into(), value);
                f.save();
            }
            _ => {
                fs::write(f.owned.root().join("turbo.json"), r#"{"futureFlags":{"experimentalSetup":true,"experimentalCargoWorkspaces":true}}"#).unwrap();
                fs::write(f.owned.root().join("Cargo.toml"), "must not probe").unwrap();
            }
        }
        let before = snapshot(f.owned.root().parent().unwrap());
        let error = f.run(&u, &flags).unwrap_err();
        if case == 0 {
            assert!(
                error
                    .to_string()
                    .contains("frozen setup requires turbo.lock")
            );
        }
        if matches!(case, 7 | 8) {
            assert!(matches!(error, Error::NotImplemented));
        }
        assert_eq!(snapshot(f.owned.root().parent().unwrap()), before);
        assert_eq!(u.hits(), 0);
        assert!(!f.owned.root().join(".turbo").exists());
        f.no_execution();
    }
}

const PROPOSE: &[&str] = &["--plan", "--tools-only", "--no-frozen"];
const SETUP: &[&str] = &["--tools-only", "--no-frozen"];

fn loopback(f: &Fixture, u: &LoopbackServer) -> provision::Transports {
    use turborepo_setup::{
        node_provision::NodeTransport, pnpm_provision::PnpmTransport,
        registry_resolution::RegistryTransport,
    };
    provision::Transports {
        tool_store: Some(f.owned.home().to_owned()),
        node: NodeTransport::loopback_http_for_tests(u.origin()).unwrap(),
        pnpm: PnpmTransport::loopback_http_for_tests(u.origin()).unwrap(),
        registry: RegistryTransport::loopback_http_for_tests(u.origin()).unwrap(),
        uv: turborepo_setup::uv_resolution::UvTransport::loopback_http_for_tests(u.origin())
            .unwrap(),
        rustup: turborepo_setup::rustup_provision::RustupTransport::loopback_http_for_tests(
            u.origin(),
        )
        .unwrap(),
        bun: turborepo_setup::bun_provision::BunTransport::loopback_http_for_tests(u.origin())
            .unwrap(),
        python: turborepo_setup::python_resolution::PythonTransport::loopback_http_for_tests(
            u.origin(),
        )
        .unwrap(),
        artifact: None,
        go: turborepo_setup::go_resolution::GoTransport::loopback_http_for_tests(u.origin())
            .unwrap(),
    }
}

impl Fixture {
    /// The real parser, request normalization and plan dispatch, captured.
    fn proposed(&self, upstream: &LoopbackServer, flags: &[&str]) -> Result<String, Error> {
        self.proposed_in(upstream, flags, Default::default())
    }

    /// Captures sources like setup does when `ecosystems` are enabled.
    fn proposed_in(
        &self,
        upstream: &LoopbackServer,
        flags: &[&str],
        ecosystems: turborepo_setup::lock::Ecosystems,
    ) -> Result<String, Error> {
        let mut report = None;
        crate::cli::dispatch_setup(&self.args(flags), |args, setup_args| {
            let request = SetupRequest::new(setup_args, turborepo_ci::is_ci())?;
            validate_request(&request)?;
            let discovery = root::Discovery::capture(args, self.git.clone())?;
            let sources =
                turborepo_setup::lock::Snapshot::capture_with(self.owned.root(), ecosystems)?;
            let transports = Some(loopback(self, upstream));
            report = Some(plan::report(
                &discovery,
                sources,
                &request,
                &dependencies::Dependencies::tools_only(),
                transports,
                || {
                    Ok(self
                        .owned
                        .policy_at(&self.owned.root().join("apps/web/src"))?)
                },
            )?);
            Ok(0)
        })
        .unwrap()?;
        Ok(trees(&report.unwrap()))
    }

    /// Byte-for-byte unchanged files around the repository and no execution.
    fn readonly<T>(&self, plan: impl FnOnce() -> T) -> T {
        let before = snapshot(self.owned.root().parent().unwrap());
        let result = plan();
        assert_eq!(snapshot(self.owned.root().parent().unwrap()), before);
        self.no_execution();
        result
    }

    /// Tool lines of the locked report for what setup actually wrote.
    fn locked_tools(&self) -> String {
        let report = self.plan_report(false).unwrap();
        let lines: Vec<_> = report.lines().collect();
        lines[1..].join("\n")
    }
}

fn not_reused(tools: &str, tool: Option<&str>, action: &str) -> String {
    let mut lines = Vec::new();
    let mut dropping = false;
    for line in tools.lines() {
        if dropping && line.trim_start().starts_with("Installed: ") {
            continue;
        }
        dropping = false;
        match line.strip_suffix(": reuse") {
            Some(name) if tool.is_none_or(|tool| tool == name.trim_start()) => {
                lines.push(format!("{name}: {action}"));
                dropping = true;
            }
            _ => lines.push(line.to_owned()),
        }
    }
    lines.join("\n")
}

#[test]
fn proposed_creation_drift_and_refresh_match_what_setup_then_writes() {
    let f = super::first_lock::fresh();
    fs::write(f.owned.root().join(".nvmrc"), "24.x").unwrap();
    let u = LoopbackServer::new(super::first_lock::routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
    let created = f.readonly(|| f.proposed(&u, PROPOSE).unwrap());
    assert_eq!(u.hits(), 4); // Node index and checksums, pnpm metadata and archive.
    assert_eq!(f.readonly(|| f.run(&u, PROPOSE).unwrap()), 0);
    assert!(!f.owned.root().join(".turbo").exists());
    assert_eq!(f.run(&u, SETUP).unwrap(), 0);
    let missing = "install (missing installation)";
    let tools = not_reused(&f.locked_tools(), None, missing);
    assert_eq!(
        created,
        format!(
            "No turbo.lock yet. Setup would create one with these \
             versions:\n{tools}\n\nturbo.lock changes\n  node: locked 24.0.0\n  pnpm: locked \
             10.0.0",
        )
    );

    // Targeted drift: only pnpm resolves; the locked Node is reused untouched.
    fs::write(
        f.owned.root().join("package.json"),
        r#"{"packageManager":"pnpm@10.1.0"}"#,
    )
    .unwrap();
    let update = LoopbackServer::new(super::first_lock::routes(&f, "25.0.0", "10.1.0"), |p| {
        assert!(!p.starts_with("/dist/"))
    })
    .unwrap();
    let drifted = f.readonly(|| f.proposed(&update, PROPOSE).unwrap());
    assert_eq!(update.hits(), 2);
    assert_eq!(f.run(&update, SETUP).unwrap(), 0);
    let stale = "install (not in current installation)";
    let tools = not_reused(&f.locked_tools(), Some("pnpm 10.1.0"), stale);
    assert_eq!(
        drifted,
        format!(
            "Setup would update turbo.lock to these versions:\n{tools}\n\nturbo.lock changes\n  \
             pnpm: 10.0.0 -> 10.1.0",
        )
    );

    // Explicit refresh proposes the newer Node; a repeat proposes nothing.
    let newer =
        LoopbackServer::new(super::first_lock::routes(&f, "24.1.0", "10.1.0"), |_| {}).unwrap();
    let mut refresh = PROPOSE.to_vec();
    refresh.push("--update-lock");
    let refreshed = f.readonly(|| f.proposed(&newer, &refresh).unwrap());
    assert_eq!(newer.hits(), 4);
    let mut write = SETUP.to_vec();
    write.push("--update-lock");
    assert_eq!(f.run(&newer, &write).unwrap(), 0);
    // Only Node is installed; pnpm's tree is reused beside any Node.
    let tools = not_reused(&f.locked_tools(), Some("node 24.1.0"), stale);
    assert_eq!(
        refreshed,
        format!(
            "Setup would update turbo.lock to these versions:\n{tools}\n\nturbo.lock changes\n  \
             node: 24.0.0 -> 24.1.0",
        )
    );
    let unchanged = f.readonly(|| f.proposed(&newer, &refresh).unwrap());
    assert_eq!(
        unchanged,
        format!("turbo.lock is already up to date:\n{}", f.locked_tools())
    );
}

#[test]
fn proposed_first_lock_resolves_rust_alongside_node_and_pnpm() {
    use super::rustup::{FLAGS, TOOLCHAIN, rust_routes, triple};
    // Node, pnpm, and Rust declarations without turbo.lock, as in this repository.
    let f = super::first_lock::fresh();
    let root = f.owned.root();
    fs::write(root.join("turbo.json"), FLAGS).unwrap();
    fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
    fs::write(root.join("rust-toolchain.toml"), TOOLCHAIN).unwrap();
    let installer = format!("/rustup/archive/1.29.1/{}/rustup-init", triple());
    let mut routes = super::first_lock::routes(&f, "24.0.0", "10.0.0");
    routes.extend(rust_routes("never run\n", &installer, "1.99.0"));
    let u = LoopbackServer::new(routes, |_| {}).unwrap();
    // The real dispatch captures Rust declarations under the Cargo flag.
    assert_eq!(f.readonly(|| f.run(&u, PROPOSE).unwrap()), 0);
    let rust = turborepo_setup::lock::Ecosystems {
        rust: true,
        ..Default::default()
    };
    let report = f.readonly(|| f.proposed_in(&u, PROPOSE, rust).unwrap());
    assert!(
        report.starts_with("No turbo.lock yet. Setup would create one with these versions:\n"),
        "{report}"
    );
    for tool in ["node 24.0.0", "pnpm 10.0.0", "rust 1.99.0", "rustup 1.29.1"] {
        assert!(
            report.contains(&format!("\n  {tool}: install (missing installation)\n")),
            "{tool}: {report}"
        );
    }
    let lines: Vec<_> = report.lines().collect();
    for change in ["rust: locked 1.99.0", "rustup: locked 1.29.1"] {
        assert!(
            lines.contains(&format!("  {change}").as_str()),
            "{change}: {report}"
        );
    }
    assert!(!root.join(".turbo").exists());
}

#[test]
fn proposed_plans_fail_offline_and_force_requires_a_lock_without_traffic() {
    let f = super::first_lock::fresh();
    let u = LoopbackServer::new([], |_| {}).unwrap();
    for extra in [&["--offline"][..], &["--update-lock", "--offline"]] {
        let mut flags = PROPOSE.to_vec();
        flags.extend(extra);
        let error = f.readonly(|| f.proposed(&u, &flags).unwrap_err());
        assert!(error.to_string().contains("offline resolution"), "{error}");
    }
    let mut force = PROPOSE.to_vec();
    force.push("--force");
    let error = f.readonly(|| f.proposed(&u, &force).unwrap_err());
    assert!(error.to_string().contains("requires turbo.lock"), "{error}");
    assert_eq!(u.hits(), 0);
    assert!(!f.owned.root().join(".turbo").exists());
}

#[test]
fn plan_rejects_drift_of_original_discovery_sources_and_policy_before_reporting() {
    for case in 0..7 {
        let f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        let discovery = root::Discovery::capture(&f.args(PLAN), f.git.clone()).unwrap();
        let sources = f.sources().unwrap();
        let calls = std::cell::Cell::new(0);
        let edited = std::cell::RefCell::new(None);
        let result = plan::inspect(&discovery, sources, false, f.owned.home(), || {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                let (path, contents) = match case {
                    0 => (f.owned.root().join("turbo.json"), "{}".into()),
                    1 => (f.owned.root().join("turbo.lock"), format!("{} ", f.lock)),
                    2 => (f.owned.root().join(".nvmrc"), "26.0.0".into()),
                    3 => (f.owned.root().join("apps/web/turbo.json"), ENABLED.into()),
                    4 => (
                        f.owned.root().join("apps/web/pnpm-workspace.yaml"),
                        "packages: []".into(),
                    ),
                    5 => (
                        f.owned.root().join("apps/web/.git"),
                        "gitdir: /not-probed".into(),
                    ),
                    _ => (f.owned.root().join("turbo.json"), UNSUPPORTED_SETUP.into()),
                };
                fs::write(path, contents).unwrap();
                *edited.borrow_mut() = Some(snapshot(f.owned.root().parent().unwrap()));
            }
            Ok(f.owned.policy_at(&f.owned.root().join("apps/web/src"))?)
        });
        assert!(result.is_err(), "case {case}");
        assert_eq!(calls.get(), 2);
        assert_eq!(
            snapshot(f.owned.root().parent().unwrap()),
            edited.into_inner().unwrap()
        );
        assert_eq!(u.hits(), 0);
        f.no_execution();
    }
}

#[test]
fn describe_pairs_dev_engines_and_labels_multi_part_artifacts() {
    let part =
        |url: &str, sha: &str| json!({"url":url,"sha256":sha,"format":"tar-gz","executables":{}});
    let tool: turborepo_setup::lock::Tool = serde_json::from_value(json!({
        "adapter":"pnpm","version":"12.6.0",
        "declarations":[
            {"file":"package.json","field":"/devEngines/packageManager/name","request":"pnpm"},
            {"file":"package.json","field":"/devEngines/packageManager/version","request":"12.6.0"},
            {"file":"package.json","field":"/devEngines/packageManager/onFail","request":"download"},
        ],
        "installation":{"kind":"managed","artifacts":{
            "macos-arm64":{"native":part("https://n.example/exe.tgz","aa"),"package":part("https://n.example/pnpm.tgz","bb")},
            "linux-x64-gnu":{"distribution":part("https://n.example/one.tgz","cc")},
        }},
    }))
    .unwrap();
    let render = |platform| plan::describe("pnpm", &tool, &platform, "reuse", None).join("\n");
    assert_eq!(
        render(turborepo_setup::lock::Platform::MacosArm64),
        "pnpm 12.6.0: reuse\n\
         \x20 Declaration: package.json#/devEngines/packageManager (pnpm@12.6.0)\n\
         \x20 Declaration: package.json#/devEngines/packageManager/onFail (download)\n\
         \x20 Source (native): https://n.example/exe.tgz\n\
         \x20 SHA-256 (native): aa\n\
         \x20 Source (package): https://n.example/pnpm.tgz\n\
         \x20 SHA-256 (package): bb"
    );
    assert!(
        render(turborepo_setup::lock::Platform::LinuxX64Gnu)
            .ends_with("\n  Source: https://n.example/one.tgz\n  SHA-256: cc")
    );
}
