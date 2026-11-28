use super::*;

pub(super) fn request(id: &str, version: &str, from: Option<&str>) -> CatalogRequest {
    CatalogRequest {
        id: id.into(),
        version: version.into(),
        from: from.map(str::to_owned),
        file: "turbo.json".into(),
    }
}

fn bundled() -> Catalog<'static> {
    Catalog::new(vec![Registry::aqua()])
}

#[test]
fn every_bundled_name_is_a_tool_id_for_a_bundled_recipe() {
    use turborepo_turbo_json::{RawRootTurboJson, RawTurboJson};

    let registry = Registry::aqua();
    let names = registry.names();
    assert!(names.len() > 500, "{}", names.len());
    let recipes = registry.recipes().unwrap();
    let mut missing = Vec::new();
    for (name, package) in names {
        let source = format!("aqua:{package}");
        if registry_package(&source).is_none() || recipes.recipe(package).is_none() {
            missing.push(format!("{name}\t{package}"));
        }
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));

    let rejected: Vec<_> = names
        .keys()
        .filter_map(|name| {
            let config = serde_json::json!({
                "futureFlags": {"experimentalSetup": true},
                "setup": {"additionalTools": {name: "1.0.0"}},
            });
            let parsed = RawRootTurboJson::parse(&config.to_string(), "turbo.json").unwrap();
            Some(RawTurboJson::try_from(parsed).err()?.to_string())
        })
        .collect();
    assert!(rejected.is_empty(), "{}", rejected.join("\n"));
}

#[test]
fn every_bundled_recipe_plans_without_panicking_into_artifacts_the_lock_accepts() {
    use turborepo_setup::lock::{Artifact, Document, Lock, SCHEMA_VERSION};

    let yaml = String::from_utf8(registry::bundled(1 << 30).unwrap()).unwrap();
    // aqua-registry's template lexer slices by character index, so a tag with
    // a non-ASCII character can panic it. Planning one version renders only
    // that version's templates. This covers the ones it does not reach.
    for tag in yaml.split("{{").skip(1) {
        let tag = tag.split("}}").next().unwrap();
        assert!(tag.is_ascii(), "non-ASCII template tag {{{{{tag}}}}}");
    }
    let registry = Registry::aqua();
    let recipes = registry.recipes().unwrap();
    assert!(recipes.all().count() > 2000, "{}", recipes.all().count());
    let request = request("tool", "1.2.3", None);
    let mut installable = 0;
    for (id, recipe) in recipes.all() {
        eprintln!("{id}");
        let plan = Plan::of(recipe, "aqua:example/tool".into(), &request.version);
        for release in plan.releases {
            let artifacts: BTreeMap<_, _> = release
                .platforms
                .into_iter()
                .filter_map(|(platform, planned)| {
                    let planned = planned.ok()?;
                    let artifact = Artifact {
                        url: planned.url,
                        sha256: Some("0".repeat(64)),
                        sha512: None,
                        format: turborepo_setup::lock::additional::lock_format(planned.format),
                        alternates: BTreeMap::new(),
                        root_prefix: None,
                        destination: None,
                        executables: planned.executables,
                    };
                    Some((platform, artifact))
                })
                .collect();
            if artifacts.is_empty() {
                continue;
            }
            installable += 1;
            let lock = Lock::new(Document {
                schema_version: SCHEMA_VERSION,
                tools: BTreeMap::from([(
                    request.id.clone(),
                    request.resolved(&plan.source, artifacts),
                )]),
            });
            assert!(lock.is_ok(), "{id} as {}: {lock:?}", release.name);
        }
    }
    assert!(installable > 2000, "{installable}");
}

#[test]
fn bundled_lookups_answer_as_the_recipe_reader_answers() {
    use aqua_registry::{AquaPackage, ParsedRegistry, RegistryYaml};

    let yaml = String::from_utf8(registry::bundled(1 << 30).unwrap()).unwrap();
    let reader = ParsedRegistry::parse_yaml(&yaml).unwrap();
    let registry = Registry::aqua();
    let recipes = registry.recipes().unwrap();
    let identity = |recipe: &AquaPackage| {
        let repository = (recipe.repo_owner.clone(), recipe.repo_name.clone());
        (recipe.name.clone(), repository, recipe.path.clone())
    };
    let mut asked = 0;
    for row in RegistryYaml::from_yaml(&yaml).unwrap().packages {
        let recipe = &row.package;
        let repository = format!("{}/{}", recipe.repo_owner, recipe.repo_name);
        let ids = [recipe.name.clone(), Some(repository), recipe.path.clone()];
        for id in ids.into_iter().flatten().chain(row.aliases) {
            let expected = reader.package(&id).ok().as_ref().map(identity);
            let found = recipes.recipe(&id).map(|(_, recipe)| identity(recipe));
            assert_eq!(found, expected, "{id}");
            asked += 1;
        }
    }
    assert!(asked > 2500, "{asked}");
}

#[test]
fn bundled_recipes_plan_real_tools() {
    let catalog = bundled();
    let plan = catalog.plan(&request("jq", "1.8.1", None)).unwrap();
    assert_eq!(plan.source, "aqua:jqlang/jq");
    let names: Vec<_> = plan.releases.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["jq-1.8.1", "jq-v1.8.1", "v1.8.1", "1.8.1"]);
    let linux = plan.releases[0].platforms[&turborepo_setup::lock::Platform::LinuxX64Gnu]
        .as_ref()
        .unwrap();
    assert_eq!(
        linux.url,
        "https://github.com/jqlang/jq/releases/download/jq-1.8.1/jq-linux-amd64"
    );
    assert_eq!(linux.format, turborepo_turbo_json::ArtifactFormat::Binary);
    assert_eq!(
        linux.executables,
        BTreeMap::from([("jq".to_owned(), "jq".to_owned())])
    );
}

#[test]
fn a_release_named_with_a_prefix_and_a_v_is_planned() {
    let plan = bundled().plan(&request("kustomize", "5.8.3", None));
    let plan = plan.unwrap();
    let release = plan.releases.iter().find(|r| r.name == "kustomize/v5.8.3");
    let linux = release.unwrap().platforms[&turborepo_setup::lock::Platform::LinuxX64Gnu].clone();
    assert_eq!(
        linux.unwrap().url,
        "https://github.com/kubernetes-sigs/kustomize/releases/download/kustomize/v5.8.3/\
         kustomize_v5.8.3_linux_amd64.tar.gz"
    );
}

#[test]
fn unknown_names_and_packages_say_what_to_write() {
    let catalog = bundled();
    let error = catalog.plan(&request("zzz-no-such-tool", "1.0.0", None));
    let message = error.unwrap_err().to_string();
    assert!(
        message.contains("\"zzz-no-such-tool\" is not a tool name"),
        "{message}"
    );
    assert!(
        message.contains("\"from\": \"aqua:<package ID>\""),
        "{message}"
    );
    assert!(message.contains("\"artifacts\""), "{message}");

    let error = catalog.plan(&request(
        "fixture",
        "1.0.0",
        Some("aqua:example/no-such-entry"),
    ));
    let message = error.unwrap_err().to_string();
    assert!(
        message.contains("no entry \"example/no-such-entry\""),
        "{message}"
    );
}

const ACME: &str = r#"
packages:
  - name: acme-cli
    type: github_release
    repo_owner: acme
    repo_name: cli
    asset: acme-cli-{{.OS}}-{{.Arch}}
    format: raw
    aliases:
      - name: acme-old
  - name: raw
    type: http
    url: https://downloads.example.test/acme/raw-{{.Version}}-{{.OS}}-{{.Arch}}
    format: raw
  - name: team/deploy-tool
    type: http
    url: https://downloads.example.test/acme/deploy-{{.OS}}-{{.Arch}}
    format: raw
"#;

/// The registry `acme` of a repository, with this text.
fn declared(yaml: &[u8]) -> Registry<'_> {
    Registry::declared("acme", "tools/registry.yaml", |_| Ok(yaml.to_vec()))
}

fn acme() -> Registry<'static> {
    declared(ACME.as_bytes())
}

fn aqua() -> Registry<'static> {
    let names = "raw\texample/raw\nhttp-tool\texample/http-tool\n";
    Registry::aqua_for_tests(names, plan::tests::REGISTRY)
}

fn linux_url(plan: &Plan) -> String {
    let linux = &plan.releases[0].platforms[&turborepo_setup::lock::Platform::LinuxX64Gnu];
    linux.as_ref().unwrap().url.clone()
}

#[test]
fn the_first_registry_that_knows_a_tool_id_answers() {
    let plan = |catalog: &Catalog, id: &str| catalog.plan(&request(id, "1.0.0", None)).unwrap();

    let in_front = Catalog::new(vec![acme(), aqua()]);
    let shadowed = plan(&in_front, "raw");
    assert_eq!(shadowed.source, "acme:raw");
    assert_eq!(
        linux_url(&shadowed),
        "https://downloads.example.test/acme/raw-v1.0.0-linux-amd64"
    );
    assert_eq!(plan(&in_front, "acme-cli").source, "acme:acme-cli");
    assert_eq!(
        plan(&in_front, "http-tool").source,
        "aqua:example/http-tool"
    );

    let behind = Catalog::new(vec![aqua(), acme()]);
    let built_in = plan(&behind, "raw");
    assert_eq!(built_in.source, "aqua:example/raw");
    assert_eq!(
        linux_url(&built_in),
        "https://github.com/example/raw/releases/download/v1.0.0/raw-linux-amd64"
    );
    assert_eq!(plan(&behind, "acme-cli").source, "acme:acme-cli");

    let alone = Catalog::new(vec![acme()]);
    assert_eq!(plan(&alone, "raw").source, "acme:raw");
    let error = alone.plan(&request("http-tool", "1.0.0", None));
    assert!(matches!(error, Err(Error::UnknownName { .. })), "{error:?}");
}

#[test]
fn a_lookup_by_name_records_the_package_that_answered_and_from_is_recorded_as_written() {
    let catalog = Catalog::new(vec![acme(), aqua()]);
    let alias = catalog.plan(&request("acme-old", "1.0.0", None)).unwrap();
    assert_eq!(alias.source, "acme:acme-cli");
    let canonical = catalog.plan(&request("acme-cli", "1.0.0", None)).unwrap();
    assert_eq!(alias, canonical);
    assert_eq!(
        linux_url(&alias),
        "https://github.com/acme/cli/releases/download/v1.0.0/acme-cli-linux-amd64"
    );
    let executables = &alias.releases[0].platforms[&turborepo_setup::lock::Platform::LinuxX64Gnu];
    let executables = &executables.as_ref().unwrap().executables;
    assert_eq!(executables.keys().collect::<Vec<_>>(), ["acme-cli"]);

    for from in ["acme:acme-old", "acme:acme-cli"] {
        let named = catalog.plan(&request("tool", "1.0.0", Some(from)));
        let named = named.unwrap();
        assert_eq!(named.source, from);
        assert_eq!(named.releases, canonical.releases);
    }

    // A short name of the built-in registry can stand for an alias.
    let names = "old\telsewhere/old-name\n";
    let built_in = Catalog::new(vec![Registry::aqua_for_tests(names, plan::tests::REGISTRY)]);
    let moved = built_in.plan(&request("old", "1.0.0", None)).unwrap();
    assert_eq!(moved.source, "aqua:example/moved");

    let unnamed =
        "packages:\n  - name: acme tools/cli~\n    type: http\n    url: https://e.test/cli\n    \
         aliases:\n      - name: cli\n";
    let catalog = Catalog::new(vec![declared(unnamed.as_bytes()), aqua()]);
    let error = catalog.plan(&request("cli", "1.0.0", None)).unwrap_err();
    assert!(
        matches!(&error, Error::UnrecordablePackage { tool, registry }
            if tool == "cli" && registry == "acme"),
        "{error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("the registry \"acme\" knows \"cli\"") && message.contains("package ID"),
        "{message}"
    );
}

#[test]
fn from_reaches_only_the_registry_it_names() {
    let catalog = Catalog::new(vec![acme(), aqua()]);
    let plan = |from: &str| catalog.plan(&request("tool", "1.0.0", Some(from)));

    let built_in = plan("aqua:example/raw").unwrap();
    assert_eq!(built_in.source, "aqua:example/raw");
    assert!(linux_url(&built_in).starts_with("https://github.com/example/raw/"));
    assert_eq!(plan("acme:raw").unwrap().source, "acme:raw");
    let nested = plan("acme:team/deploy-tool").unwrap();
    assert_eq!(nested.source, "acme:team/deploy-tool");

    for (from, registry, package) in [
        ("acme:example/raw", "acme", "example/raw"),
        ("aqua:acme-cli", "aqua", "acme-cli"),
        ("aqua:raw", "aqua", "raw"),
    ] {
        let error = plan(from).unwrap_err();
        assert!(
            matches!(&error, Error::UnknownPackage { registry: r, package: p, .. }
                if r == registry && p == package),
            "{from}: {error:?}"
        );
        assert!(
            error.to_string().contains(&format!("{registry:?}")),
            "{error}"
        );
    }

    for (catalog, from) in [
        (Catalog::new(vec![acme(), aqua()]), "other:raw"),
        (Catalog::new(vec![acme()]), "aqua:example/raw"),
        (Catalog::new(Vec::new()), "aqua:example/raw"),
    ] {
        let error = catalog
            .plan(&request("tool", "1.0.0", Some(from)))
            .unwrap_err();
        assert!(
            matches!(&error, Error::UnknownRegistry { tool, from: named }
                if tool == "tool" && named == from),
            "{from}: {error:?}"
        );
    }
}

#[test]
fn an_unknown_name_lists_the_registries_that_were_asked_in_order() {
    let catalog = Catalog::new(vec![acme(), aqua()]);
    let error = catalog
        .plan(&request("deploy-tool", "2.0.0", None))
        .unwrap_err();
    assert!(
        matches!(&error, Error::UnknownName { tool, registries, candidates, .. }
            if tool == "deploy-tool"
                && registries == &["acme".to_owned(), "aqua".to_owned()]
                && candidates == &["acme:team/deploy-tool".to_owned()]),
        "{error:?}"
    );
    let message = error.to_string();
    for part in [
        "\"deploy-tool\" is not a tool name",
        "in order: acme, aqua",
        "\"acme:team/deploy-tool\"",
        "{ \"version\": \"2.0.0\", \"from\": \"<registry>:<package ID>\" }",
        "{ \"version\": \"2.0.0\", \"artifacts\": { ... } }",
    ] {
        assert!(message.contains(part), "{part}: {message}");
    }

    let both = Catalog::new(vec![aqua(), acme()]);
    let error = both.plan(&request("xz", "5.8.1", None)).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("in order: aqua, acme"), "{message}");
    assert!(message.contains("\"aqua:example/xz\""), "{message}");

    let alone = Catalog::new(vec![acme()]);
    let message = alone
        .plan(&request("xz", "5.8.1", None))
        .unwrap_err()
        .to_string();
    assert!(message.contains("the registry \"acme\" knows"), "{message}");
    assert!(
        message.contains("\"from\": \"acme:<package ID>\""),
        "{message}"
    );
    assert!(!message.contains("aqua"), "{message}");
}

/// The registry `acme` with this text, counting how often it is read.
fn counted<'a>(yaml: &'a [u8], reads: &'a std::cell::Cell<usize>) -> Registry<'a> {
    Registry::declared("acme", "tools/registry.yaml", move |_| {
        reads.set(reads.get() + 1);
        Ok(yaml.to_vec())
    })
}

#[test]
fn a_lookup_reads_only_the_registries_it_asks_and_each_at_most_once() {
    const BROKEN: &[u8] = b"packages: [";
    let plan = |catalog: &Catalog, id: &str, from: Option<&str>| {
        let plan = catalog.plan(&request(id, "1.0.0", from));
        plan.map(|plan| plan.source)
    };
    let names_acme = |error: Error| {
        assert!(
            matches!(&error, Error::Registry { registry, origin, problem: Problem::Malformed { .. } }
                if registry == "acme" && origin == "tools/registry.yaml"),
            "{error:?}"
        );
    };

    let reads = std::cell::Cell::new(0);
    let behind = Catalog::new(vec![aqua(), counted(BROKEN, &reads)]);
    assert_eq!(plan(&behind, "raw", None).unwrap(), "aqua:example/raw");
    let from = Some("aqua:example/http-tool");
    assert_eq!(
        plan(&behind, "tool", from).unwrap(),
        "aqua:example/http-tool"
    );
    assert_eq!(reads.get(), 0);
    names_acme(plan(&behind, "acme-cli", None).unwrap_err());
    names_acme(plan(&behind, "tool", Some("acme:acme-cli")).unwrap_err());
    assert_eq!(reads.get(), 1);

    let reads = std::cell::Cell::new(0);
    let in_front = Catalog::new(vec![counted(BROKEN, &reads), aqua()]);
    assert_eq!(
        plan(&in_front, "tool", from).unwrap(),
        "aqua:example/http-tool"
    );
    assert_eq!(reads.get(), 0);
    names_acme(plan(&in_front, "raw", None).unwrap_err());
    names_acme(plan(&in_front, "raw", None).unwrap_err());
    assert_eq!(reads.get(), 1);

    let reads = std::cell::Cell::new(0);
    let working = Catalog::new(vec![counted(ACME.as_bytes(), &reads), aqua()]);
    assert_eq!(reads.get(), 0);
    assert_eq!(plan(&working, "acme-cli", None).unwrap(), "acme:acme-cli");
    assert_eq!(
        plan(&working, "http-tool", None).unwrap(),
        "aqua:example/http-tool"
    );
    assert_eq!(
        plan(&working, "tool", Some("acme:raw")).unwrap(),
        "acme:raw"
    );
    assert!(plan(&working, "nothing", None).is_err());
    assert_eq!(reads.get(), 1);

    // The built-in registry is read when its short names know the tool or
    // `from` names it, and not before.
    let unreadable = || Registry::aqua_for_tests("raw\texample/raw\n", "packages: [");
    let names_aqua = |error: Error| {
        let message = error.to_string();
        let expected = "cannot use the registry \"aqua\" (built into turbo): ";
        assert!(message.starts_with(expected), "{message}");
    };
    let in_front = Catalog::new(vec![unreadable(), acme()]);
    assert_eq!(plan(&in_front, "acme-cli", None).unwrap(), "acme:acme-cli");
    assert_eq!(
        plan(&in_front, "tool", Some("acme:raw")).unwrap(),
        "acme:raw"
    );
    names_aqua(plan(&in_front, "raw", None).unwrap_err());
    let behind = Catalog::new(vec![acme(), unreadable()]);
    assert_eq!(plan(&behind, "raw", None).unwrap(), "acme:raw");
    names_aqua(plan(&behind, "tool", Some("aqua:example/raw")).unwrap_err());
}

#[test]
fn a_registry_whose_source_fails_says_how() {
    use std::io::ErrorKind;

    for (kind, expected, shown) in [
        (ErrorKind::NotFound, Problem::Missing, "it does not exist"),
        (
            ErrorKind::FileTooLarge,
            Problem::TooLarge(MAX_REGISTRY_BYTES),
            "it is larger than 16 MiB",
        ),
        (
            ErrorKind::PermissionDenied,
            Problem::Unreadable("permission denied".into()),
            "it cannot be read: permission denied",
        ),
    ] {
        let registry = Registry::declared("acme", "tools/registry.yaml", move |limit| {
            assert_eq!(limit, MAX_REGISTRY_BYTES);
            Err(kind.into())
        });
        let catalog = Catalog::new(vec![registry]);
        let error = catalog.plan(&request("tool", "1.0.0", None)).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("cannot use the registry \"acme\" (tools/registry.yaml): {shown}")
        );
        assert!(
            matches!(&error, Error::Registry { problem, .. } if problem == &expected),
            "{error:?}"
        );
    }
}

/// Why the registry `acme` with these bytes cannot be used.
fn problem(bytes: &[u8]) -> Problem {
    let catalog = Catalog::new(vec![declared(bytes)]);
    let Err(error) = catalog.plan(&request("tool", "1.0.0", None)) else {
        panic!(
            "the registry was accepted: {}",
            String::from_utf8_lossy(bytes)
        );
    };
    let message = error.to_string();
    assert!(
        message.starts_with("cannot use the registry \"acme\" (tools/registry.yaml): "),
        "{message}"
    );
    match error {
        Error::Registry {
            registry, problem, ..
        } if registry == "acme" => problem,
        error => panic!("{error:?}"),
    }
}

#[test]
fn registry_bytes_are_refused_by_name_before_the_recipe_reader_sees_them() {
    let large = vec![b'#'; MAX_REGISTRY_BYTES + 1];
    assert_eq!(problem(&large), Problem::TooLarge(MAX_REGISTRY_BYTES));
    assert_eq!(problem(&large).to_string(), "it is larger than 16 MiB");
    assert_eq!(
        problem(b"packages:\n  - name: \xff\xfe\n"),
        Problem::NotUtf8
    );
    assert_eq!(problem(b"packages: []\n"), Problem::NoPackages);
    for (yaml, line) in [
        ("packages: [\n  - name: {", 2),
        ("packages: 7\n", 1),
        ("packages:\n  - name: tool\n    type: unknown\n", 3),
    ] {
        let found = problem(yaml.as_bytes());
        assert!(
            matches!(found, Problem::Malformed { at: Some(Position { line: at, .. }) } if at == line),
            "{yaml}: {found:?}"
        );
    }

    let at_the_limit = [
        ACME.as_bytes(),
        &vec![b'\n'; MAX_REGISTRY_BYTES - ACME.len()],
    ]
    .concat();
    let catalog = Catalog::new(vec![declared(&at_the_limit)]);
    assert!(catalog.plan(&request("acme-cli", "1.0.0", None)).is_ok());
}

#[test]
fn an_error_about_a_registry_says_where_its_fault_is_and_never_shows_its_text() {
    const MARKER: &str = "MARKER-s3cret";
    for (yaml, line) in [
        (format!("packages:\n  - name: {MARKER}\n   bad: [\n"), 3),
        (format!("packages: \"{MARKER}\"\n"), 1),
        (
            format!("packages:\n  - name: tool\n    type: {MARKER}\n"),
            3,
        ),
        (format!("{MARKER}: [1, 2]\n"), 1),
        (
            format!("packages:\n  - name: tool\n    {MARKER}: !!int {MARKER}\n"),
            3,
        ),
    ] {
        let catalog = Catalog::new(vec![declared(yaml.as_bytes())]);
        let error = catalog.plan(&request("tool", "1.0.0", None)).unwrap_err();
        let message = error.to_string();
        assert!(
            message.starts_with("cannot use the registry \"acme\" (tools/registry.yaml): ")
                && message.contains(&format!(" (line {line}, column ")),
            "{message}"
        );
        assert!(!message.contains(MARKER), "{message}");
        assert!(!format!("{error:?}").contains(MARKER), "{error:?}");
    }
}
