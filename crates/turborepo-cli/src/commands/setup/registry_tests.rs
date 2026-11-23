use turborepo_setup_catalog::{MAX_REGISTRY_BYTES, Problem};

use super::*;

const PATH: &str = "tools/registry.yaml";
const ACME: &str = r#"
packages:
  - name: acme-fixture
    type: github_release
    repo_owner: example
    repo_name: fixture
    asset: fixture-{{trimV .Version}}-{{.OS}}-{{.Arch}}.tar.gz
    files:
      - name: fixture
        src: fixture-{{trimV .Version}}/bin/fixture
    aliases:
      - name: fixture
"#;

fn acme() -> Value {
    json!({"name": "acme", "path": PATH})
}

fn configure(f: &Fixture, registries: Value, tools: Value) {
    let config = json!({"futureFlags": {"experimentalSetup": true},
        "setup": {"registries": registries, "additionalTools": tools}});
    fs::write(f.owned.root().join("turbo.json"), config.to_string()).unwrap();
}

fn registry_file(f: &Fixture, contents: &[u8]) {
    let path = f.owned.root().join(PATH);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn acme_fixture() -> (Fixture, LoopbackServer) {
    let (f, u) = fixture();
    configure(
        &f,
        json!([acme(), "aqua"]),
        json!({"acme-fixture": "1.2.3"}),
    );
    registry_file(&f, ACME.as_bytes());
    (f, u)
}

fn source(f: &Fixture, id: &str) -> Value {
    let locked: Value = serde_json::from_slice(&lock(f)).unwrap();
    locked["tools"][id]["options"]["source"].clone()
}

#[test]
fn a_repository_registry_resolves_and_locks_its_tool_and_is_not_read_again() {
    let (f, u) = acme_fixture();
    assert_eq!(run(&f, &u, LOCAL).unwrap(), 0);
    assert_eq!(source(&f, "acme-fixture"), json!(["acme:acme-fixture"]));
    let host = serde_json::to_value(provision::platform().unwrap()).unwrap();
    let locked: Value = serde_json::from_slice(&lock(&f)).unwrap();
    let artifacts = &locked["tools"]["acme-fixture"]["installation"]["artifacts"];
    assert_eq!(
        artifacts[host.as_str().unwrap()]["artifact"]["url"],
        format!("https://github.com{RELEASE}/{}", asset())
    );
    let current = Installed::open(&f.owned).current().unwrap().unwrap();
    let output = std::process::Command::new(current.bin.join("fixture"))
        .output()
        .unwrap();
    assert_eq!(output.stdout, b"fixture 1.2.3\n");

    let (bytes, hits) = (lock(&f), u.hits());
    assert_eq!(run(&f, &u, LOCAL).unwrap(), 0);
    assert_eq!(u.hits(), hits);
    assert_eq!(lock(&f), bytes);

    // Nothing is left to resolve, so the registry file is not needed.
    for contents in [Some(&b"packages: {{\xff"[..]), None] {
        match contents {
            Some(contents) => registry_file(&f, contents),
            None => fs::remove_file(f.owned.root().join(PATH)).unwrap(),
        }
        for flags in [LOCAL, FROZEN, &["--plan", "--tools-only"]] {
            assert_eq!(run(&f, &u, flags).unwrap(), 0, "{flags:?}");
        }
        assert_eq!(u.hits(), hits);
        assert_eq!(lock(&f), bytes);
    }
    f.no_execution();
}

#[test]
fn registries_are_asked_in_the_order_turbo_json_declares() {
    for (registries, expected) in [
        (json!([acme(), "aqua"]), "acme:acme-fixture"),
        (json!(["aqua", acme()]), "aqua:example/fixture"),
        (json!([acme()]), "acme:acme-fixture"),
    ] {
        let (f, u) = fixture();
        configure(&f, registries.clone(), json!({"fixture": "1.2.3"}));
        registry_file(&f, ACME.as_bytes());
        assert_eq!(run(&f, &u, LOCAL).unwrap(), 0, "{registries}");
        assert_eq!(source(&f, "fixture"), json!([expected]), "{registries}");
    }
}

#[test]
fn a_tool_found_through_an_alias_is_reused_when_from_names_its_package() {
    let (f, u) = fixture();
    configure(&f, json!([acme(), "aqua"]), json!({"fixture": "1.2.3"}));
    registry_file(&f, ACME.as_bytes());
    assert_eq!(run(&f, &u, LOCAL).unwrap(), 0);
    assert_eq!(source(&f, "fixture"), json!(["acme:acme-fixture"]));
    let (bytes, hits) = (lock(&f), u.hits());

    let from = json!({"fixture": {"version": "1.2.3", "from": "acme:acme-fixture"}});
    configure(&f, json!([acme(), "aqua"]), from);
    for flags in [LOCAL, FROZEN] {
        assert_eq!(run(&f, &u, flags).unwrap(), 0, "{flags:?}");
        assert_eq!(u.hits(), hits, "{flags:?}");
        assert_eq!(lock(&f), bytes, "{flags:?}");
    }

    // The alias is another entry, so naming it resolves again.
    let alias = json!({"fixture": {"version": "1.2.3", "from": "acme:fixture"}});
    configure(&f, json!([acme(), "aqua"]), alias);
    let error = run(&f, &u, FROZEN).unwrap_err();
    assert!(matches!(&error, Error::AdditionalToolDrift(_)), "{error}");
    assert_eq!(run(&f, &u, LOCAL).unwrap(), 0);
    assert_eq!(source(&f, "fixture"), json!(["acme:fixture"]));
    f.no_execution();
}

#[test]
fn removing_a_registry_is_drift_when_frozen_and_unresolved_locally() {
    let (f, u) = acme_fixture();
    assert_eq!(run(&f, &u, LOCAL).unwrap(), 0);
    let (bytes, hits) = (lock(&f), u.hits());

    for registries in [json!(["aqua"]), json!([])] {
        configure(&f, registries.clone(), json!({"acme-fixture": "1.2.3"}));
        for flags in [FROZEN, &["--tools-only", "--no-frozen", "--force"]] {
            let error = run(&f, &u, flags).unwrap_err();
            assert!(
                matches!(&error, Error::AdditionalToolDrift(ids)
                    if ids.iter().map(String::as_str).collect::<Vec<_>>() == ["acme-fixture"]),
                "{registries} {flags:?}: {error}"
            );
        }
        let error = run(&f, &u, LOCAL).unwrap_err();
        let message = error.to_string();
        assert!(
            matches!(
                &error,
                Error::Catalog(turborepo_setup_catalog::Error::UnknownName { .. })
            ) && message.contains("\"acme-fixture\""),
            "{registries}: {message}"
        );
        assert_eq!(u.hits(), hits, "{registries}");
        assert_eq!(lock(&f), bytes, "{registries}");
    }

    configure(&f, json!([acme()]), json!({"acme-fixture": "1.2.3"}));
    assert_eq!(run(&f, &u, FROZEN).unwrap(), 0);
    assert_eq!(lock(&f), bytes);
}

/// The fault of the registry `acme`, which the error names with its file.
fn registry_problem(error: &Error) -> &Problem {
    let message = error.to_string();
    assert!(
        message.starts_with(&format!("cannot use the registry \"acme\" ({PATH}): ")),
        "{message}"
    );
    match error {
        Error::Catalog(turborepo_setup_catalog::Error::Registry {
            registry, problem, ..
        }) if registry == "acme" => problem,
        error => panic!("{error:?}"),
    }
}

#[test]
fn an_unusable_registry_file_fails_by_name_before_anything_changes() {
    let (f, u) = acme_fixture();
    let bytes = lock(&f);
    fs::remove_file(f.owned.root().join(PATH)).unwrap();
    let error = run(&f, &u, LOCAL).unwrap_err();
    assert_eq!(registry_problem(&error), &Problem::Missing);

    let tag = "packages:\n  - name: acme-fixture\n    type: http\n    url: \
               \"https://example.test/{{\u{e9} .Version}}/bad\"\n    format: raw\n";
    for contents in [
        tag.as_bytes(),
        b"packages:\n  - name: \xff\n",
        b"packages: [",
        b"API_TOKEN=s3cret-value\npackages: s3cret-value\n",
        b"packages:\n  - name: acme-fixture\n    type: s3cret-value\n",
    ] {
        registry_file(&f, contents);
        let error = run(&f, &u, LOCAL).unwrap_err();
        registry_problem(&error);
        let message = error.to_string();
        assert!(
            !message.contains("s3cret") && !message.contains('\u{e9}'),
            "{message}"
        );
    }

    registry_file(&f, &vec![b'\n'; MAX_REGISTRY_BYTES + 1]);
    let error = run(&f, &u, LOCAL).unwrap_err();
    let too_large = Problem::TooLarge(MAX_REGISTRY_BYTES);
    assert_eq!(registry_problem(&error), &too_large);
    assert!(error.to_string().ends_with("it is larger than 16 MiB"));

    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("registry.yaml"), ACME).unwrap();
        let tools = f.owned.root().join("tools");
        fs::remove_dir_all(&tools).unwrap();
        std::os::unix::fs::symlink(outside.path(), &tools).unwrap();
        let error = run(&f, &u, LOCAL).unwrap_err();
        let problem = registry_problem(&error);
        assert!(matches!(problem, Problem::Unreadable(_)), "{problem:?}");
    }

    assert_eq!(u.hits(), 0);
    assert_eq!(lock(&f), bytes);
    assert_eq!(f.manifest(), None);
    f.no_execution();
}

#[test]
fn a_registry_file_is_read_only_when_a_lookup_asks_its_registry() {
    let by_name = json!({"fixture": "1.2.3"});
    let from_aqua = json!({"fixture": {"version": "1.2.3", "from": "aqua:example/fixture"}});
    for contents in [None, Some(&b"packages: ["[..])] {
        for (registries, tools, asked) in [
            (json!(["aqua", acme()]), &by_name, false),
            (json!(["aqua", acme()]), &from_aqua, false),
            (json!([acme(), "aqua"]), &from_aqua, false),
            (json!([acme(), "aqua"]), &by_name, true),
        ] {
            let (f, u) = fixture();
            configure(&f, registries.clone(), tools.clone());
            if let Some(contents) = contents {
                registry_file(&f, contents);
            }
            let case = format!("{registries} {tools} {contents:?}");
            let ran = run(&f, &u, LOCAL);
            if asked {
                registry_problem(&ran.unwrap_err());
                assert_eq!(u.hits(), 0, "{case}");
                continue;
            }
            assert_eq!(ran.unwrap(), 0, "{case}");
            assert_eq!(
                source(&f, "fixture"),
                json!(["aqua:example/fixture"]),
                "{case}"
            );
        }
    }
}
