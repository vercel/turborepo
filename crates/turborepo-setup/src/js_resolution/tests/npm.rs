use super::*;

fn npm_bytes(version: &str) -> Vec<u8> {
    tar(&[
        (
            "package/package.json",
            json!({"name":"npm","version":version,
            "bin":{"npm":"bin/npm-cli.js","npx":"bin/npx-cli.js"},
            "scripts":{"install":"exit 99"}})
            .to_string(),
        ),
        (
            "package/bin/npm-cli.js",
            "throw new Error('must not execute');".into(),
        ),
        (
            "package/bin/npx-cli.js",
            "throw new Error('must not execute');".into(),
        ),
    ])
}
fn npm_routes(version: &str, bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    vec![
        (
            format!("/npm/{version}"),
            json!({"name":"npm","version":version,"dist":{
            "tarball":format!("https://registry.npmjs.org/npm/-/npm-{version}.tgz"),
            "integrity":format!("sha512-{}", STANDARD.encode(Sha512::digest(bytes)))}})
            .to_string()
            .into_bytes(),
        ),
        (format!("/npm/-/npm-{version}.tgz"), bytes.to_vec()),
    ]
}
fn npm_fixture(version: &str) -> World {
    let mut routes = node_routes(&["24.0.0"]);
    routes.extend(npm_routes(version, &npm_bytes(version)));
    World::new(routes, |_| {})
}
fn canonical(lock: &Lock, root: &Path) {
    let bytes = lock.canonical_bytes().unwrap();
    assert_eq!(Lock::parse(&bytes).unwrap(), *lock);
    assert!(
        lock.matches_native(Snapshot::capture(root).unwrap().declarations())
            .unwrap()
    );
    let text = String::from_utf8(bytes).unwrap();
    for private in ["localhost", "127.0.0.1", root.to_str().unwrap()] {
        assert!(!text.contains(private));
    }
}
fn node_identity(lock: &Lock) -> Tool {
    let mut node = lock.tools()["node"].clone();
    remove_node_npm_exports(&mut node);
    node
}

#[test]
fn first_selection_bundled_independent_and_authored_bytes_repeat_without_traffic() {
    for (version, algorithm) in [
        ("11.6.1", None),
        ("10.0.0", None),
        ("11.6.1", Some("sha256")),
        ("11.6.1", Some("sha512")),
        ("11.6.1", Some("dual")),
    ] {
        let bytes = npm_bytes(version);
        let suffix = match algorithm {
            Some("sha256" | "dual") => format!("+sha256.{:x}", Sha256::digest(&bytes)),
            Some("sha512") => format!("+sha512.{:x}", Sha512::digest(&bytes)),
            _ => String::new(),
        };
        let dev = if algorithm == Some("dual") {
            format!("{version}+sha512.{:x}", Sha512::digest(&bytes))
        } else {
            version.into()
        };
        let repo = root(
            Some("24.x"),
            json!({"packageManager":format!("npm@{version}{suffix}"),
            "devEngines":{"packageManager":{"name":"npm","version":dev}},
            "scripts":{"build":"touch task-ran"}}),
        );
        fs::write(repo.path().join("package-lock.json"), "dependency sentinel").unwrap();
        let world = npm_fixture(version);
        let result = apply(repo.path(), Mode::Local, false, &world).unwrap().lock;
        canonical(&result, repo.path());
        assert_eq!(
            fs::read(repo.path().join("package-lock.json")).unwrap(),
            b"dependency sentinel"
        );
        assert!(!repo.path().join(".turbo/tools").exists());
        assert!(!repo.path().join("task-ran").exists());
        let bundled = version == "11.6.1" && algorithm.is_none();
        let npm = &result.tools()["npm"];
        assert_eq!(npm.version, version);
        assert_eq!(npm.adapter, "npm");
        let Installation::Managed { artifacts } = &result.tools()["node"].installation else {
            panic!()
        };
        assert_eq!(artifacts.len(), 6);
        for platform in PLATFORMS {
            NodePlan::from_lock(&result, platform).unwrap();
            assert_eq!(
                artifacts[&platform]["distribution"].executables.len(),
                if bundled { 3 } else { 1 }
            );
        }
        let mut paths = vec![
            "/dist/index.json".into(),
            "/dist/v24.0.0/SHASUMS256.txt".into(),
        ];
        if bundled {
            assert_eq!(
                npm.installation,
                Installation::Bundled {
                    owner: "node".into()
                }
            );
        } else {
            let Installation::Managed { artifacts } = &npm.installation else {
                panic!()
            };
            assert_eq!(artifacts.len(), 1);
            let artifact = &artifacts[&Platform::Any]["package"];
            assert_eq!(artifact.sha256, format!("{:x}", Sha256::digest(&bytes)));
            assert_eq!(
                artifact.executables,
                BTreeMap::from([
                    ("npm".into(), "bin/npm-cli.js".into()),
                    ("npx".into(), "bin/npx-cli.js".into())
                ])
            );
            assert!(result.tools()["node"].options.is_empty());
            paths.extend([
                format!("/npm/{version}"),
                format!("/npm/-/npm-{version}.tgz"),
            ]);
        }
        assert_eq!(world.paths(), paths);
        for mode in [Mode::Local, Mode::Frozen, Mode::NoLock] {
            let silent = World::new(vec![], |_| {});
            assert_eq!(
                apply(repo.path(), mode, true, &silent).unwrap().lock,
                result
            );
            assert!(silent.paths().is_empty());
        }
    }
    let repo = root(
        Some("24.x"),
        json!({"devEngines":{"packageManager":[
        {"name":"npm","version":"11.6.1"},{"name":"npm","version":"11.6.1"}]}}),
    );
    let world = npm_fixture("11.6.1");
    assert!(matches!(
        apply(repo.path(), Mode::NoLock, false, &world)
            .unwrap()
            .lock
            .tools()["npm"]
            .installation,
        Installation::Bundled { .. }
    ));
    assert_eq!(
        world.paths(),
        ["/dist/index.json", "/dist/v24.0.0/SHASUMS256.txt"]
    );
}

#[test]
fn manager_switch_changes_only_permitted_exports_and_removal_never_refreshes_node() {
    let repo = root(Some("24.x"), json!({}));
    let p = repo.path();
    let original = seed(p);
    let mut old = original.clone();
    for (manager, version, traffic) in [
        ("npm", "11.6.1", false),
        ("npm", "10.0.0", true),
        ("npm", "11.6.1", true),
        ("pnpm", "10.0.0", true),
        ("npm", "10.0.0", true),
    ] {
        write_manifest(p, json!({"packageManager":format!("{manager}@{version}")}));
        let routes = if manager == "npm" {
            npm_routes(version, &npm_bytes(version))
        } else {
            registry_routes(&pnpm_bytes())
        };
        let world = World::new(routes, |_| {}); // No Node endpoint, even for a bundled switch.
        let next = apply(p, Mode::Local, false, &world).unwrap().lock;
        assert_eq!(node_identity(&next), node_identity(&original));
        assert_eq!(next.tools().len(), 2);
        assert_eq!(next.tools()[manager].version, version);
        if manager == "pnpm" {
            assert_eq!(next.tools()["node"], old.tools()["node"]);
        }
        let paths = if traffic {
            vec![
                format!("/{manager}/{version}"),
                format!("/{manager}/-/{manager}-{version}.tgz"),
            ]
        } else {
            vec![]
        };
        assert_eq!(world.paths(), paths);
        canonical(&next, p);
        old = next;
    }
    write_manifest(p, json!({}));
    let world = World::new(vec![], |_| {});
    let removed = apply(p, Mode::Local, true, &world).unwrap().lock;
    assert_eq!(
        removed.tools(),
        &BTreeMap::from([("node".into(), old.tools()["node"].clone())])
    );
    assert!(world.paths().is_empty());
    canonical(&removed, p);
}

#[test]
fn legacy_node_without_bundled_identity_uses_independent_npm_without_guessing() {
    let repo = root(Some("24.x"), json!({}));
    let mut previous = seed(repo.path()).document().clone();
    remove_node_npm_exports(previous.tools.get_mut("node").unwrap());
    let previous = Lock::new(previous).unwrap();
    save(repo.path(), &previous);
    write_manifest(repo.path(), json!({"packageManager":"npm@11.6.1"}));
    let world = World::new(npm_routes("11.6.1", &npm_bytes("11.6.1")), |_| {});
    let next = apply(repo.path(), Mode::Local, false, &world).unwrap().lock;
    assert_eq!(next.tools()["node"], previous.tools()["node"]);
    assert!(matches!(
        next.tools()["npm"].installation,
        Installation::Managed { .. }
    ));
    assert_eq!(world.paths(), ["/npm/11.6.1", "/npm/-/npm-11.6.1.tgz"]);
}

#[test]
fn node_drift_preserves_independent_npm_all_variants_and_authoritative_floating_reuse() {
    for (version, request) in [
        ("10.0.0", "npm@10.0.0"),
        ("10.0.0", "npm@10.x"),
        ("11.6.1", "npm@11.6.1"),
    ] {
        let repo = root(
            Some("24.x"),
            json!({"packageManager":format!("npm@{version}+sha256.{:x}", Sha256::digest(npm_bytes(version)))}),
        );
        let old = apply(repo.path(), Mode::Local, false, &npm_fixture(version))
            .unwrap()
            .lock;
        write_manifest(repo.path(), json!({"packageManager":request}));
        let declarations = Snapshot::capture(repo.path()).unwrap().declarations()["npm"].clone();
        let mut old = old.document().clone();
        let npm = old.tools.get_mut("npm").unwrap();
        npm.declarations = declarations;
        let Installation::Managed { artifacts } = &mut npm.installation else {
            panic!()
        };
        *artifacts = PLATFORMS
            .into_iter()
            .map(|p| (p, artifacts[&Platform::Any].clone()))
            .collect();
        let old = Lock::new(old).unwrap();
        save(repo.path(), &old);
        fs::write(repo.path().join(".nvmrc"), "^24.0.0").unwrap();
        let world = World::new(node_routes(&["24.1.0"]), |_| {});
        let next = apply(repo.path(), Mode::Local, false, &world).unwrap().lock;
        assert_eq!(next.tools()["npm"], old.tools()["npm"]);
        assert_eq!(next.tools()["node"].version, "24.1.0");
        assert!(next.tools()["node"].options.is_empty());
        assert_eq!(
            world.paths(),
            ["/dist/index.json", "/dist/v24.1.0/SHASUMS256.txt"]
        );
        canonical(&next, repo.path());
    }
}

#[test]
fn unsupported_native_choices_fail_before_traffic_and_preserved_npm_preflights_every_variant() {
    for manifest in [
        json!({"packageManager":"npm@11.x"}),
        json!({"devEngines":{"packageManager":{"name":"npm"}}}),
        json!({"devEngines":{"packageManager":[{"name":"npm","version":"10.0.0"},{"name":"npm","version":"11.6.1"}]}}),
        json!({"packageManager":format!("npm@11.6.1+sha1.{}", "a".repeat(40))}),
        json!({"packageManager":"npm@11.6.1","devEngines":{"packageManager":[
            {"name":"npm","version":format!("11.6.1+sha512.{}", "a".repeat(128))},
            {"name":"npm","version":"11.6.1"}]}}),
    ] {
        let repo = root(Some("24.x"), manifest);
        rejected(repo.path(), &World::new(vec![], |_| {}), false, &[]);
    }
    for (node, offline) in [(None, false), (Some("24.x"), true)] {
        let repo = root(node, json!({"packageManager":"npm@11.6.1"}));
        rejected(repo.path(), &World::new(vec![], |_| {}), offline, &[]);
    }
    for (field, bad) in [
        ("url", json!("https://example.invalid/npm.tgz")),
        ("format", json!("zip")),
        ("rootPrefix", json!("wrong")),
        ("destination", json!("nested")),
        ("executables", json!({"npm":"bin/npm-cli.js"})),
        ("options", json!({"opaque":["x"]})),
        ("adapter", json!("pnpm")),
        (
            "system",
            json!({"kind":"verify-system","executables":["npm","npx"]}),
        ),
    ] {
        let repo = root(Some("24.x"), json!({"packageManager":"npm@10.0.0"}));
        let old = apply(repo.path(), Mode::NoLock, false, &npm_fixture("10.0.0"))
            .unwrap()
            .lock;
        let mut document = serde_json::to_value(old.document()).unwrap();
        let tool = &mut document["tools"]["npm"];
        let package = tool["installation"]["artifacts"]["any"].clone();
        tool["installation"]["artifacts"] = json!({"macos-arm64":package,"windows-arm64":package});
        match field {
            "options" | "adapter" => tool[field] = bad,
            "system" => tool["installation"] = bad,
            _ => tool["installation"]["artifacts"]["windows-arm64"]["package"][field] = bad,
        }
        let old = Lock::parse(&serde_json::to_vec(&document).unwrap()).unwrap();
        save(repo.path(), &old);
        fs::write(repo.path().join(".nvmrc"), "^24.0.0").unwrap();
        rejected(
            repo.path(),
            &World::new(node_routes(&["24.1.0"]), |_| {}),
            false,
            &[],
        );
    }
    // Missing artifact integrity cannot even enter the captured resolver.
    let repo = root(Some("24.x"), json!({"packageManager":"npm@10.0.0"}));
    let old = apply(repo.path(), Mode::Local, false, &npm_fixture("10.0.0"))
        .unwrap()
        .lock;
    let mut document = serde_json::to_value(old.document()).unwrap();
    document["tools"]["npm"]["installation"]["artifacts"]["any"]["package"]
        .as_object_mut()
        .unwrap()
        .remove("sha256");
    fs::write(
        repo.path().join("turbo.lock"),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();
    assert!(Snapshot::capture(repo.path()).is_err());
}

#[test]
fn preserved_native_node_and_authored_npm_integrity_fail_before_either_transport() {
    let repo = root(
        Some("24.x"),
        json!({"packageManager":format!("npm@11.6.1+sha256.{:x}", Sha256::digest(npm_bytes("11.6.1")))}),
    );
    let old = apply(repo.path(), Mode::NoLock, false, &npm_fixture("11.6.1"))
        .unwrap()
        .lock;
    let mut document = old.document().clone();
    let Installation::Managed { artifacts } =
        &mut document.tools.get_mut("npm").unwrap().installation
    else {
        panic!()
    };
    artifacts
        .get_mut(&Platform::Any)
        .unwrap()
        .get_mut("package")
        .unwrap()
        .sha256 = "b".repeat(64);
    save(repo.path(), &Lock::new(document).unwrap());
    fs::write(repo.path().join(".nvmrc"), "^24.0.0").unwrap();
    rejected(
        repo.path(),
        &World::new(node_routes(&["24.1.0"]), |_| {}),
        false,
        &[],
    );

    let repo = root(Some("24.x"), json!({}));
    let mut document = serde_json::to_value(seed(repo.path()).document()).unwrap();
    document["tools"]["node"]["version"] = json!("23.0.0");
    document["tools"]["node"]["installation"] = serde_json::from_str(
        &document["tools"]["node"]["installation"]
            .to_string()
            .replace("24.0.0", "23.0.0"),
    )
    .unwrap();
    save(
        repo.path(),
        &Lock::parse(&serde_json::to_vec(&document).unwrap()).unwrap(),
    );
    write_manifest(repo.path(), json!({"packageManager":"npm@10.0.0"}));
    rejected(
        repo.path(),
        &World::new(npm_routes("10.0.0", &npm_bytes("10.0.0")), |_| {}),
        false,
        &[],
    );
}

#[test]
fn registry_mismatch_integrity_and_drift_never_publish_partial_cohorts() {
    for failure in [
        "sha256",
        "sha512",
        "registry-integrity",
        "missing-integrity",
        "identity",
        "package",
        "bytes",
        "drift",
    ] {
        let repo = root(Some("24.x"), json!({}));
        let p = repo.path();
        seed(p);
        let pin = match failure {
            "sha256" => format!("+sha256.{}", "a".repeat(64)),
            "sha512" => format!("+sha512.{}", "a".repeat(128)),
            _ => String::new(),
        };
        write_manifest(p, json!({"packageManager":format!("npm@11.6.1{pin}")}));
        let bytes = if failure == "package" {
            npm_bytes("10.0.0")
        } else {
            npm_bytes("11.6.1")
        };
        let mut routes = npm_routes("11.6.1", &bytes);
        if matches!(
            failure,
            "registry-integrity" | "missing-integrity" | "identity"
        ) {
            let mut metadata: Value = serde_json::from_slice(&routes[0].1).unwrap();
            match failure {
                "identity" => metadata["version"] = json!("10.0.0"),
                "missing-integrity" => {
                    metadata["dist"]
                        .as_object_mut()
                        .unwrap()
                        .remove("integrity");
                }
                _ => metadata["dist"]["integrity"] = json!("sha1-bad"),
            }
            routes[0].1 = serde_json::to_vec(&metadata).unwrap();
        } else if failure == "bytes" {
            routes[1].1 = b"corrupt".to_vec();
        }
        // No authored pin would use bundled npm, so force the independent path
        // while retaining the same authoritative Node release/artifact bytes.
        if pin.is_empty() {
            let mut old = Snapshot::capture(p)
                .unwrap()
                .previous_lock()
                .unwrap()
                .document()
                .clone();
            remove_node_npm_exports(old.tools.get_mut("node").unwrap());
            save(p, &Lock::new(old).unwrap());
        }
        let path = p.join("package.json");
        let world = World::new(routes, move |request| {
            if failure == "drift" && request.ends_with(".tgz") {
                fs::write(&path, "{}").unwrap();
            }
        });
        let paths = if matches!(
            failure,
            "registry-integrity" | "missing-integrity" | "identity"
        ) {
            vec!["/npm/11.6.1"]
        } else {
            vec!["/npm/11.6.1", "/npm/-/npm-11.6.1.tgz"]
        };
        rejected(p, &world, false, &paths);
    }
}

#[test]
fn changing_node_bundled_identity_cannot_silently_replace_unaffected_npm_pin() {
    let repo = root(Some("24.x"), json!({"packageManager":"npm@11.6.1"}));
    let old = apply(repo.path(), Mode::NoLock, false, &npm_fixture("11.6.1"))
        .unwrap()
        .lock;
    save(repo.path(), &old);
    fs::write(repo.path().join(".nvmrc"), "^24.0.0").unwrap();
    let mut routes = node_routes(&["24.1.0"]);
    let mut index: Value = serde_json::from_slice(&routes[0].1).unwrap();
    index[0]["npm"] = json!("11.7.0");
    routes[0].1 = serde_json::to_vec(&index).unwrap();
    rejected(
        repo.path(),
        &World::new(routes, |_| {}),
        false,
        &["/dist/index.json", "/dist/v24.1.0/SHASUMS256.txt"],
    );
    assert_eq!(
        lock_bytes(repo.path()).unwrap(),
        old.canonical_bytes().unwrap()
    );
}
