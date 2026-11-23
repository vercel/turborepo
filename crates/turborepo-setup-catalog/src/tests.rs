use super::*;

pub(super) fn request(id: &str, version: &str, from: Option<&str>) -> CatalogRequest {
    CatalogRequest {
        id: id.into(),
        version: version.into(),
        from: from.map(str::to_owned),
        file: "turbo.json".into(),
    }
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
    let catalog = Catalog::bundled();
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
    let plan = Catalog::bundled().plan(&request("kustomize", "5.8.3", None));
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
    let catalog = Catalog::bundled();
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

#[test]
fn a_lookup_by_name_records_the_package_that_answered() {
    // A short name of the built-in registry can stand for an alias.
    let names = "old\telsewhere/old-name\n";
    let built_in = Catalog::for_tests(names, plan::tests::REGISTRY);
    let moved = built_in.plan(&request("old", "1.0.0", None)).unwrap();
    assert_eq!(moved.source, "aqua:example/moved");
}
