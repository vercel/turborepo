use super::*;
use crate::lock::reconcile::propose;

const JQ: &str = "aqua:jqlang/jq";
const JJ: &str = "aqua:jj-vcs/jj";

fn catalog() -> Value {
    json!({"jq": "1.8.2", "jj": {"version": "0.47.1", "from": JJ}})
}

fn acme() -> Value {
    json!({"name": "acme", "path": "tools/registry.yaml"})
}

fn declare(root: &Path, registries: Value, tools: &Value) {
    let config = json!({"futureFlags": {"experimentalSetup": true},
        "setup": {"registries": registries, "additionalTools": tools}});
    fs::write(root.join("turbo.json"), config.to_string()).unwrap();
}

fn lock_resolved(root: &Path, sources: &[(&str, &str)]) -> Vec<u8> {
    let snapshot = capture(root);
    let requests = requests(&snapshot);
    let tools = sources
        .iter()
        .map(|(id, source)| ((*id).to_owned(), resolved(&requests[*id], source)))
        .collect();
    let snapshot = snapshot.with_catalog_tools(tools).unwrap();
    let outcome = reconcile(&snapshot, Mode::Local, false, never).unwrap();
    assert_eq!(outcome.publication, Some(WriteOutcome::Written));
    lock_bytes(root)
}

fn capture(root: &Path) -> Snapshot {
    Snapshot::capture_with(root, Ecosystems::default()).unwrap()
}

fn requests(snapshot: &Snapshot) -> BTreeMap<String, CatalogRequest> {
    snapshot
        .catalog_requests()
        .map(|request| (request.id.clone(), request.clone()))
        .collect()
}

fn resolved(request: &CatalogRequest, source: &str) -> Tool {
    let artifact = |target: &str| Artifact {
        url: format!(
            "https://downloads.example.test/{}-{target}.tar.gz",
            request.id
        ),
        sha256: Some(SHA.into()),
        sha512: None,
        format: Format::TarGz,
        alternates: BTreeMap::new(),
        root_prefix: None,
        destination: None,
        executables: BTreeMap::from([(
            request.id.clone(),
            format!("{}-{target}/{}", request.id, request.id),
        )]),
    };
    request.resolved(
        source,
        BTreeMap::from([
            (Platform::LinuxX64Gnu, artifact("linux")),
            (Platform::MacosArm64, artifact("macos")),
        ]),
    )
}

fn locked(tools: &Value) -> tempfile::TempDir {
    let root = repo(tools);
    let snapshot = capture(root.path());
    let resolved = requests(&snapshot)
        .iter()
        .map(|(id, request)| {
            let source = request.from.clone().unwrap_or(format!("aqua:example/{id}"));
            (id.clone(), resolved(request, &source))
        })
        .collect();
    let snapshot = snapshot.with_catalog_tools(resolved).unwrap();
    let outcome = reconcile(&snapshot, Mode::Local, false, never).unwrap();
    assert_eq!(outcome.publication, Some(WriteOutcome::Written));
    root
}

fn parts(tool: &mut Tool) -> &mut BTreeMap<String, Artifact> {
    let Installation::Managed { artifacts } = &mut tool.installation else {
        panic!("catalog tools are managed");
    };
    artifacts.get_mut(&Platform::LinuxX64Gnu).unwrap()
}

fn second_part(parts: &mut BTreeMap<String, Artifact>) {
    let mut extra = parts[PART].clone();
    extra.executables = BTreeMap::from([("jq-extra".into(), "jq-linux/jq-extra".into())]);
    parts.insert("extra".into(), extra);
}

fn ids(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

#[test]
fn unresolved_declarations_never_publish_and_resolved_ones_lock_their_source() {
    let mut tools = catalog();
    tools["fixture"] = tool();
    let root = repo(&tools);
    let snapshot = capture(root.path());
    assert_eq!(
        snapshot.additional_tools().keys().collect::<Vec<_>>(),
        ["fixture"]
    );
    let requests = requests(&snapshot);
    assert_eq!(
        requests["jq"],
        CatalogRequest {
            id: "jq".into(),
            version: "1.8.2".into(),
            from: None,
            file: "turbo.json".into(),
        }
    );
    assert_eq!(requests["jj"].from.as_deref(), Some(JJ));

    for mode in [Mode::Local, Mode::Refresh, Mode::NoLock] {
        let error = reconcile(&snapshot, mode, false, never).unwrap_err();
        assert!(
            matches!(&error, Error::Unresolved(unresolved) if unresolved == &ids(&["jj", "jq"])),
            "{mode:?}: {error:?}"
        );
        assert!(error.to_string().contains("\"jq\""), "{error}");
    }
    assert!(matches!(
        propose(&snapshot, Mode::Local, false, never, |_| Ok(()), || Ok(())),
        Err(Error::Unresolved(_))
    ));
    assert!(matches!(
        reconcile(&snapshot, Mode::Frozen, false, never),
        Err(Error::Missing)
    ));
    assert!(!root.path().join("turbo.lock").exists());

    let resolved = BTreeMap::from([
        ("jq".to_owned(), resolved(&requests["jq"], JQ)),
        ("jj".to_owned(), resolved(&requests["jj"], JJ)),
    ]);
    let snapshot = snapshot.with_catalog_tools(resolved).unwrap();
    assert_eq!(snapshot.catalog_requests().count(), 0);
    let outcome = reconcile(&snapshot, Mode::Local, false, never).unwrap();
    assert_eq!(outcome.publication, Some(WriteOutcome::Written));
    snapshot.after_publication(&outcome.lock).unwrap();

    let lock: Value = serde_json::from_slice(&lock_bytes(root.path())).unwrap();
    let jq = &lock["tools"]["jq"];
    assert_eq!(jq["adapter"], ADAPTER);
    assert_eq!(jq["version"], "1.8.2");
    assert_eq!(jq["options"], json!({"source": [JQ]}));
    assert_eq!(
        jq["declarations"],
        json!([{"file": "turbo.json", "field": "/setup/additionalTools/jq", "request": "1.8.2"}])
    );
    assert_eq!(
        jq["installation"]["artifacts"]["macos-arm64"][PART],
        json!({"url": "https://downloads.example.test/jq-macos.tar.gz", "sha256": SHA,
            "format": "tar-gz", "executables": {"jq": "jq-macos/jq"}})
    );
    assert_eq!(lock["tools"]["jj"]["options"], json!({"source": [JJ]}));
    assert!(lock["tools"]["fixture"].get("options").is_none());
}

#[test]
fn a_matching_lock_entry_is_reused_without_resolution() {
    let root = locked(&catalog());
    let bytes = lock_bytes(root.path());
    let snapshot = capture(root.path());
    assert_eq!(snapshot.catalog_requests().count(), 0);
    let lock = snapshot.previous_lock().unwrap();
    assert_eq!(snapshot.additional_tools(), lock.tools());
    assert!(snapshot.matches_additional(lock));
    for (mode, publication) in [
        (Mode::Local, Some(WriteOutcome::Unchanged)),
        (Mode::Refresh, Some(WriteOutcome::Unchanged)),
        (Mode::Frozen, None),
        (Mode::NoLock, None),
    ] {
        let outcome = reconcile(&snapshot, mode, false, never).unwrap();
        assert_eq!(outcome.publication, publication, "{mode:?}");
    }
    assert_eq!(lock_bytes(root.path()), bytes);

    assert_eq!(
        lock.tools()["jq"].options[SOURCE],
        ["aqua:example/jq".to_owned()]
    );
}

#[test]
fn changed_or_missing_entries_are_unresolved_and_drift_when_frozen() {
    let mut original = catalog();
    original["fixture"] = tool();
    type Change = fn(&mut Value);
    let cases: [(&str, Change); 6] = [
        ("jq", |tools| tools["jq"] = json!("1.8.3")),
        ("jj", |tools| {
            tools["jj"]["from"] = json!("aqua:martinvonz/jj")
        }),
        ("jq", |tools| {
            tools["jq"] = json!({"version": "1.8.2", "from": JQ})
        }),
        ("fixture", |tools| tools["fixture"] = json!("1.2.3")),
        (
            "rg",
            |tools| tools["rg"] = json!({"version": "15.2.0", "from": "aqua:BurntSushi/ripgrep"}),
        ),
        ("fzf", |tools| tools["fzf"] = json!({"version": "0.70.0"})),
    ];
    for (id, change) in cases {
        let root = locked(&original);
        let bytes = lock_bytes(root.path());
        let mut tools = original.clone();
        change(&mut tools);
        config(root.path(), "turbo.json", &tools);
        let snapshot = capture(root.path());
        assert_eq!(
            requests(&snapshot).keys().collect::<Vec<_>>(),
            [id],
            "{tools}"
        );
        assert!(!snapshot.additional_tools().contains_key(id));
        let lock = snapshot.previous_lock().unwrap();
        assert_eq!(snapshot.additional_drift(lock), ids(&[id]));
        assert!(!snapshot.matches_additional(lock));
        for mode in [Mode::Frozen, Mode::NoLock] {
            let error = reconcile(&snapshot, mode, false, never).unwrap_err();
            assert!(
                matches!(&error, Error::Drift(drift) if drift == &ids(&[id])),
                "{id} {mode:?}: {error:?}"
            );
        }
        assert!(matches!(
            reconcile(&snapshot, Mode::Local, false, never),
            Err(Error::Unresolved(unresolved)) if unresolved == ids(&[id])
        ));
        assert_eq!(lock_bytes(root.path()), bytes);
    }

    let root = locked(&catalog());
    fs::rename(
        root.path().join("turbo.json"),
        root.path().join("turbo.jsonc"),
    )
    .unwrap();
    assert_eq!(
        requests(&capture(root.path())).keys().collect::<Vec<_>>(),
        ["jj", "jq"]
    );
}

#[test]
fn an_entry_that_lacks_a_target_platform_is_reused_and_offered_for_another_resolution() {
    let root = locked(&catalog());
    let bytes = lock_bytes(root.path());
    let snapshot = capture(root.path());
    assert_eq!(snapshot.catalog_requests().count(), 0);
    let incomplete: BTreeMap<_, _> = snapshot
        .incomplete_catalog_requests()
        .map(|request| (request.id.clone(), request.clone()))
        .collect();
    assert_eq!(incomplete.keys().collect::<Vec<_>>(), ["jj", "jq"]);
    assert_eq!(incomplete["jj"].from.as_deref(), Some(JJ));
    assert_eq!(incomplete["jq"].from, None);
    for mode in [Mode::Local, Mode::Refresh, Mode::Frozen, Mode::NoLock] {
        reconcile(&snapshot, mode, false, never).unwrap();
    }
    assert_eq!(lock_bytes(root.path()), bytes);

    let mut complete = resolved(&incomplete["jq"], "aqua:example/jq");
    let Installation::Managed { artifacts } = &mut complete.installation else {
        panic!("catalog tools are managed");
    };
    let linux = artifacts[&Platform::LinuxX64Gnu].clone();
    let mut windows = linux.clone();
    windows.get_mut(PART).unwrap().executables =
        BTreeMap::from([("jq".into(), "jq-windows/jq.exe".into())]);
    artifacts.insert(Platform::MacosX64, linux.clone());
    artifacts.insert(Platform::LinuxArm64Gnu, linux);
    artifacts.insert(Platform::WindowsX64, windows);
    let supplied = BTreeMap::from([("jq".to_owned(), complete.clone())]);
    let snapshot = snapshot.with_catalog_tools(supplied).unwrap();
    let outcome = reconcile(&snapshot, Mode::Refresh, false, never).unwrap();
    assert_eq!(outcome.publication, Some(WriteOutcome::Written));
    assert_eq!(outcome.lock.tools()["jq"], complete);

    let snapshot = capture(root.path());
    let offered: Vec<_> = snapshot.incomplete_catalog_requests().collect();
    assert_eq!(offered, [&incomplete["jj"]]);
}

#[test]
fn a_recorded_entry_in_a_shape_resolution_never_writes_is_unresolved() {
    type Change = fn(&mut BTreeMap<String, Artifact>);
    let cases: [(&str, Change); 3] = [
        ("a built-in executable", |parts| {
            let artifact = parts.get_mut(PART).unwrap();
            let path = artifact.executables.remove("jq").unwrap();
            artifact.executables.insert("cargo".into(), path);
        }),
        ("a root prefix", |parts| {
            parts.get_mut(PART).unwrap().root_prefix = Some("jq-linux".into())
        }),
        ("two parts", second_part),
    ];
    for (case, change) in cases {
        let root = locked(&catalog());
        let mut document: Document = serde_json::from_slice(&lock_bytes(root.path())).unwrap();
        change(parts(document.tools.get_mut("jq").unwrap()));
        let bytes = Lock::new(document).unwrap().canonical_bytes().unwrap();
        fs::write(root.path().join("turbo.lock"), &bytes).unwrap();

        let snapshot = capture(root.path());
        let requests = requests(&snapshot);
        assert_eq!(requests.keys().collect::<Vec<_>>(), ["jq"], "{case}");
        let lock = snapshot.previous_lock().unwrap();
        assert_eq!(snapshot.additional_drift(lock), ids(&["jq"]), "{case}");
        for mode in [Mode::Frozen, Mode::NoLock] {
            let error = reconcile(&snapshot, mode, false, never).unwrap_err();
            assert!(
                matches!(&error, Error::Drift(drift) if drift == &ids(&["jq"])),
                "{case} {mode:?}: {error:?}"
            );
        }
        assert!(matches!(
            reconcile(&snapshot, Mode::Local, false, never),
            Err(Error::Unresolved(unresolved)) if unresolved == ids(&["jq"])
        ));
        assert_eq!(lock_bytes(root.path()), bytes, "{case}");

        let again = BTreeMap::from([("jq".to_owned(), resolved(&requests["jq"], JQ))]);
        let snapshot = snapshot.with_catalog_tools(again).unwrap();
        let outcome = reconcile(&snapshot, Mode::Local, false, never).unwrap();
        assert_eq!(outcome.publication, Some(WriteOutcome::Written), "{case}");
        assert_eq!(capture(root.path()).catalog_requests().count(), 0, "{case}");
    }

    let root = locked(&catalog());
    let mut document: Document = serde_json::from_slice(&lock_bytes(root.path())).unwrap();
    let artifact = parts(document.tools.get_mut("jq").unwrap())
        .get_mut(PART)
        .unwrap();
    artifact.sha256 = None;
    artifact.sha512 = Some("0".repeat(128));
    assert!(Lock::new(document).is_err());
}

#[test]
fn resolved_tools_that_install_one_command_are_refused_by_name() {
    let mut tools = catalog();
    tools["fixture"] = tool();
    let root = repo(&tools);
    let snapshot = capture(root.path());
    let requests = requests(&snapshot);
    let exporting = |id: &str, source: &str, command: &str| {
        let mut tool = resolved(&requests[id], source);
        let artifact = parts(&mut tool).get_mut(PART).unwrap();
        let path = artifact.executables.remove(id).unwrap();
        artifact.executables.insert(command.into(), path);
        tool
    };
    let supply = |tools: [(&str, Tool); 2]| {
        let tools = tools.map(|(id, tool)| (id.to_owned(), tool));
        snapshot.with_catalog_tools(tools.into()).unwrap_err()
    };

    let jj = resolved(&requests["jj"], JJ);
    let error = supply([("jq", exporting("jq", JQ, "jj")), ("jj", jj.clone())]);
    assert!(
        matches!(&error, StorageError::Collision { tool, other, executable }
            if tool == "jj" && other == "jq" && executable == "jj"),
        "{error:?}"
    );
    let message = error.to_string();
    for part in ["\"jj\"", "\"jq\""] {
        assert!(message.contains(part), "{message}");
    }

    let error = supply([("jq", exporting("jq", JQ, "Fixture")), ("jj", jj)]);
    assert!(
        matches!(&error, StorageError::Collision { tool, other, executable }
            if tool == "fixture" && other == "jq" && executable == "Fixture"),
        "{error:?}"
    );
}

#[test]
fn a_resolved_tool_that_does_not_match_its_request_is_refused() {
    let root = repo(&catalog());
    let snapshot = capture(root.path());
    let requests = requests(&snapshot);
    let (jq, jj) = (&requests["jq"], &requests["jj"]);
    let supply = |id: &str, tool: Tool| {
        snapshot
            .with_catalog_tools(BTreeMap::from([(id.to_owned(), tool)]))
            .map(drop)
    };
    assert!(supply("jq", resolved(jq, JQ)).is_ok());
    assert!(supply("jj", resolved(jj, JJ)).is_ok());

    let mut version = resolved(jq, JQ);
    version.version = "1.8.3".into();
    let mut request = resolved(jq, JQ);
    request.declarations[0].request = Some("1.8.3".into());
    let mut file = resolved(jq, JQ);
    file.declarations[0].file = "turbo.jsonc".into();
    let mut hand_declared = resolved(jq, JQ);
    hand_declared.options.clear();
    let mut extra = resolved(jq, JQ);
    extra.options.insert("flavor".into(), vec!["x".into()]);
    let mut source = resolved(jq, JQ);
    source
        .options
        .insert(SOURCE.into(), vec!["jqlang/jq".into()]);
    let reshaped = |change: fn(&mut Artifact)| {
        let mut tool = resolved(jq, JQ);
        change(parts(&mut tool).get_mut(PART).unwrap());
        tool
    };
    let builtin = reshaped(|artifact| {
        let path = artifact.executables.remove("jq").unwrap();
        artifact.executables.insert("cargo".into(), path);
    });
    let root_prefix = reshaped(|artifact| artifact.root_prefix = Some("jq-linux".into()));
    let destination = reshaped(|artifact| artifact.destination = Some("tools".into()));
    let sha512 = reshaped(|artifact| {
        artifact.sha256 = None;
        artifact.sha512 = Some("0".repeat(128));
    });
    let mut two_parts = resolved(jq, JQ);
    second_part(parts(&mut two_parts));
    let refused = [
        ("jq", builtin),
        ("jq", root_prefix),
        ("jq", destination),
        ("jq", sha512),
        ("jq", two_parts),
        ("jq", version),
        ("jq", request),
        ("jq", file),
        ("jq", hand_declared),
        ("jq", extra),
        ("jq", source),
        ("jq", resolved(jj, JJ)),
        ("jj", resolved(jj, "aqua:martinvonz/jj")),
        (
            "rg",
            resolved(
                &CatalogRequest {
                    id: "rg".into(),
                    ..jq.clone()
                },
                JQ,
            ),
        ),
    ];
    for (id, tool) in refused {
        let error = supply(id, tool.clone()).unwrap_err();
        assert!(
            matches!(error, StorageError::Lock(_)),
            "{id}: {tool:?} {error:?}"
        );
    }
    let resolved_jq = BTreeMap::from([("jq".to_owned(), resolved(jq, JQ))]);
    let once = snapshot.with_catalog_tools(resolved_jq.clone()).unwrap();
    assert!(once.with_catalog_tools(resolved_jq).is_err());
}

#[test]
fn an_entry_from_a_declared_repository_registry_is_reused() {
    let tools = json!({"acme-cli": "1.4.2",
        "deploy": {"version": "2.0.0", "from": "acme:deploy-tool"}});
    let root = repo(&tools);
    declare(root.path(), json!([acme(), "aqua"]), &tools);
    assert_eq!(capture(root.path()).registries().len(), 2);
    let sources = [
        ("acme-cli", "acme:acme-cli"),
        ("deploy", "acme:deploy-tool"),
    ];
    let bytes = lock_resolved(root.path(), &sources);
    let lock: Value = serde_json::from_slice(&bytes).unwrap();
    for (id, source) in sources {
        assert_eq!(lock["tools"][id]["options"], json!({"source": [source]}));
    }

    // Neither the order of the registries nor a new one in front moves a
    // tool that is already locked.
    for registries in [
        json!([acme(), "aqua"]),
        json!(["aqua", acme()]),
        json!([acme()]),
        json!([{"name": "first", "path": "first.yaml"}, acme(), "aqua"]),
    ] {
        declare(root.path(), registries.clone(), &tools);
        let snapshot = capture(root.path());
        assert_eq!(snapshot.catalog_requests().count(), 0, "{registries}");
        assert!(snapshot.matches_additional(snapshot.previous_lock().unwrap()));
        for mode in [Mode::Local, Mode::Frozen, Mode::NoLock] {
            reconcile(&snapshot, mode, false, never).unwrap();
        }
        assert_eq!(lock_bytes(root.path()), bytes, "{registries}");
    }
}

#[test]
fn an_entry_whose_registry_is_no_longer_declared_is_unresolved_and_drift_when_frozen() {
    let tools = json!({"acme-cli": "1.4.2", "jq": "1.8.2"});
    let root = repo(&tools);
    declare(root.path(), json!([acme(), "aqua"]), &tools);
    let bytes = lock_resolved(root.path(), &[("acme-cli", "acme:acme-cli"), ("jq", JQ)]);

    let renamed = json!([{"name": "other", "path": "tools/registry.yaml"}, "aqua"]);
    for (registries, dropped) in [
        (json!([acme()]), vec!["jq"]),
        (json!(["aqua"]), vec!["acme-cli"]),
        (renamed, vec!["acme-cli"]),
        (json!([]), vec!["acme-cli", "jq"]),
    ] {
        declare(root.path(), registries.clone(), &tools);
        let snapshot = capture(root.path());
        let unresolved: Vec<_> = snapshot.catalog_requests().map(|r| &*r.id).collect();
        assert_eq!(unresolved, dropped, "{registries}");
        let lock = snapshot.previous_lock().unwrap();
        assert_eq!(snapshot.additional_drift(lock), ids(&dropped));
        for mode in [Mode::Frozen, Mode::NoLock] {
            let error = reconcile(&snapshot, mode, false, never).unwrap_err();
            assert!(
                matches!(&error, Error::Drift(drift) if drift == &ids(&dropped)),
                "{registries} {mode:?}: {error:?}"
            );
            assert!(error.to_string().contains(dropped[0]), "{error}");
        }
        assert!(matches!(
            reconcile(&snapshot, Mode::Local, false, never),
            Err(Error::Unresolved(unresolved)) if unresolved == ids(&dropped)
        ));
        assert_eq!(lock_bytes(root.path()), bytes, "{registries}");
    }

    // Without `setup.registries`, only the built-in registry is declared.
    config(root.path(), "turbo.json", &tools);
    let unresolved = requests(&capture(root.path()));
    assert_eq!(unresolved.keys().collect::<Vec<_>>(), ["acme-cli"]);
}

#[test]
fn a_resolved_tool_from_an_undeclared_registry_is_refused() {
    let tools = json!({"jq": "1.8.2"});
    let root = repo(&tools);
    declare(root.path(), json!([acme()]), &tools);
    let snapshot = capture(root.path());
    let jq = &requests(&snapshot)["jq"];
    let supply = |source: &str| {
        let tools = BTreeMap::from([("jq".to_owned(), resolved(jq, source))]);
        snapshot.with_catalog_tools(tools).map(drop)
    };
    assert!(matches!(supply(JQ), Err(StorageError::Lock(_))));
    assert!(matches!(supply("other:jq"), Err(StorageError::Lock(_))));
    assert!(supply("acme:jq").is_ok());
}
