//! Playwright configs that come from a factory in a workspace package
//! (`fixtures/frameworks/playwright-factory`): `defineConfig(__dirname)` of
//! `@acme/playwright/config`. The factory is followed for Playwright only; jest
//! and vitest keep treating a package's `defineConfig(x)` as `x`.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::{
    FrameworkInputs, FrameworkUncertainty, InputRole, Target, TestPattern, framework,
    jsconfig::{self, LoadOptions},
    playwright::{self, DEFAULT_TEST_MATCH},
};
use turborepo_trace_lang_js::extract::config::ConfigVal;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/frameworks/playwright-factory")
        .canonicalize()
        .expect("fixture exists")
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or_else(|_| panic!("{} is not under {}", p.display(), root.display()))
        .to_string_lossy()
        .into_owned()
}

fn specs(root: &Path, inputs: &FrameworkInputs) -> Vec<String> {
    playwright::discover_specs(inputs)
        .iter()
        .map(|p| rel(root, p))
        .collect()
}

/// Every uncertainty of the config and its projects.
fn uncertainties(inputs: &FrameworkInputs) -> Vec<&FrameworkUncertainty> {
    inputs
        .uncertainties
        .iter()
        .chain(inputs.projects.iter().flat_map(|p| &p.uncertainties))
        .collect()
}

/// The uncertainties about what decides which files are specs.
fn discovery_uncertainties(inputs: &FrameworkInputs) -> Vec<&FrameworkUncertainty> {
    uncertainties(inputs)
        .into_iter()
        .filter(|u| {
            u.field.as_deref().is_some_and(|f| {
                ["testDir", "testMatch", "testIgnore", "projects"]
                    .iter()
                    .any(|k| f.contains(k))
            })
        })
        .collect()
}

fn globs(ps: &[TestPattern]) -> Vec<&str> {
    ps.iter()
        .map(|p| match p {
            TestPattern::Glob(g) => g.as_str(),
            TestPattern::Regex(_) => "<regex>",
        })
        .collect()
}

fn files(root: &Path, inputs: &FrameworkInputs) -> Vec<String> {
    let mut out: Vec<String> = inputs
        .projects
        .iter()
        .flat_map(|p| &p.global.config_files)
        .map(|f| rel(root, f))
        .collect();
    out.sort();
    out.dedup();
    out
}

#[test]
fn factory_of_a_workspace_package_is_followed() {
    let root = fixture();
    let (inputs, _) = playwright::load_config(&root.join("apps/docs/playwright.config.ts"));

    // The specs are the e2e files; the vitest test is not one.
    assert_eq!(
        specs(&root, &inputs),
        [
            "apps/docs/app/cls.e2e.chromium.ts",
            "apps/docs/app/navigation.e2e.ts"
        ]
    );
    assert_eq!(
        discovery_uncertainties(&inputs),
        Vec::<&FrameworkUncertainty>::new()
    );
    // What is left is what the environment decides elsewhere (`CI`, `DEVTOOLS`, the
    // base URL): notes on the factory's files, none on a field, each one once
    // (the factory is evaluated for the import and again for the call).
    let all = uncertainties(&inputs);
    for (i, u) in all.iter().enumerate() {
        assert!(!all[..i].contains(u), "twice: {u:?}");
    }
    for u in all {
        assert!(
            u.field.is_none()
                && u.kind == UncertaintyKind::EnvDependent
                && u.file.starts_with(root.join("tooling/playwright/src")),
            "{u:?}"
        );
    }

    // The factory's projects. `testDir: './'` is the config file's directory. What
    // the environment decides (`TEST_TYPE`) is united.
    let names: Vec<&str> = inputs.projects.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["chromium", "firefox", "webkit"]);
    for p in &inputs.projects {
        assert_eq!(p.root_dir, root.join("apps/docs"), "{}", p.name);
    }
    assert_eq!(
        globs(&inputs.projects[0].discovery.include),
        [
            "**/*.e2e.ts",
            "**/*.spec.ts",
            "**/*.e2e.chromium.ts",
            "**/*.spec.chromium.ts"
        ]
    );
    assert_eq!(
        globs(&inputs.projects[1].discovery.include),
        ["**/*.e2e.ts", "**/*.spec.ts"]
    );

    // `join(__dirname, '../../src', 'config/global-setup.ts')` is relative to the
    // factory.
    let entries = &inputs.projects[0].global.entries;
    let entry = |role: InputRole| {
        entries
            .iter()
            .find(|e| e.role == role)
            .map(|e| e.target.clone())
    };
    assert_eq!(
        entry(InputRole::GlobalSetup),
        Some(Target::Path(
            root.join("tooling/playwright/src/config/global-setup.ts")
        ))
    );
    assert_eq!(
        entry(InputRole::GlobalTeardown),
        Some(Target::Path(
            root.join("tooling/playwright/src/config/global-teardown.ts")
        ))
    );

    // The factory and what it imports are inputs of the config.
    assert_eq!(
        files(&root, &inputs),
        [
            "apps/docs/playwright.config.ts",
            "tooling/playwright/src/config/index.ts",
            "tooling/playwright/src/utils/get-base-url.ts"
        ]
    );
}

#[test]
fn spread_of_the_factory_result_overrides_fields() {
    let root = fixture();
    let (inputs, _) = playwright::load_config(&root.join("apps/shop/playwright.config.ts"));
    assert_eq!(specs(&root, &inputs), ["apps/shop/spec/cart.spec.ts"]);
    assert_eq!(
        discovery_uncertainties(&inputs),
        Vec::<&FrameworkUncertainty>::new()
    );
    for p in &inputs.projects {
        assert_eq!(p.root_dir, root.join("apps/shop/spec"), "{}", p.name);
    }
    // A project without its own `testMatch` gets the overridden one.
    assert_eq!(inputs.projects[1].name, "firefox");
    assert_eq!(
        globs(&inputs.projects[1].discovery.include),
        ["**/*.spec.ts"]
    );
}

#[test]
fn cts_config_requires_the_factory() {
    let root = fixture();
    let (inputs, _) = playwright::load_config(&root.join("apps/blog/playwright.config.cts"));
    assert_eq!(specs(&root, &inputs), ["apps/blog/app/home.e2e.ts"]);
    assert_eq!(
        discovery_uncertainties(&inputs),
        Vec::<&FrameworkUncertainty>::new()
    );
    assert_eq!(inputs.projects.len(), 3);
}

#[test]
fn unknown_test_match_of_a_factory_is_reported() {
    let root = fixture();
    let (inputs, _) = playwright::load_config(&root.join("apps/opaque/playwright.config.ts"));
    // `testMatch: specPattern(cwd)`, a call into an installed package.
    let unc = discovery_uncertainties(&inputs);
    assert_eq!(unc.len(), 1, "{unc:?}");
    assert_eq!(unc[0].field.as_deref(), Some("testMatch"));
    assert!(unc[0].detail.contains("specPattern(cwd)"), "{unc:?}");
    // Not no specs, and not silently: Playwright's default pattern, next to the
    // report.
    assert_eq!(inputs.projects.len(), 1);
    assert_eq!(
        globs(&inputs.projects[0].discovery.include),
        [DEFAULT_TEST_MATCH]
    );
    assert_eq!(specs(&root, &inputs), ["apps/opaque/app/search.spec.ts"]);
    // The rest of the factory's config is read.
    assert_eq!(inputs.projects[0].root_dir, root.join("apps/opaque"));
    assert_eq!(
        files(&root, &inputs),
        [
            "apps/opaque/playwright.config.ts",
            "tooling/playwright/src/config/opaque.ts"
        ]
    );
}

#[test]
fn define_config_of_an_installed_package_is_the_identity() {
    let root = fixture();
    let (inputs, _) = playwright::load_config(&root.join("apps/plain/playwright.config.ts"));
    // `node_modules/@playwright/test` is a real directory there: never evaluated.
    assert_eq!(specs(&root, &inputs), ["apps/plain/e2e/login.e2e.ts"]);
    assert_eq!(uncertainties(&inputs), Vec::<&FrameworkUncertainty>::new());
    assert_eq!(files(&root, &inputs), ["apps/plain/playwright.config.ts"]);
    assert_eq!(inputs.projects[0].root_dir, root.join("apps/plain/e2e"));
    assert_eq!(
        globs(&inputs.projects[0].discovery.include),
        ["**/*.e2e.ts"]
    );
}

#[test]
fn factory_that_cannot_be_found_is_reported() {
    let root = fixture();
    // No `node_modules` link: `defineConfig(__dirname)` is `__dirname`, a string.
    let (inputs, _) = playwright::load_config(&root.join("apps/unlinked/playwright.config.ts"));
    let unc = uncertainties(&inputs);
    assert!(
        unc.iter()
            .any(|u| u.field.is_none() && u.detail == "config does not evaluate to an object"),
        "{unc:?}"
    );
}

/// The config object of `file` and the files read, relative to the fixture.
fn loaded(root: &Path, file: &str, follow_factories: bool) -> (Vec<String>, Vec<String>, usize) {
    let l = jsconfig::load(
        &root.join(file),
        LoadOptions {
            follow_factories,
            ..LoadOptions::default()
        },
    );
    let keys = match l.value {
        Some(ConfigVal::Obj(o)) => o.entries.into_iter().map(|(k, _)| k).collect(),
        other => panic!("not an object: {other:?}"),
    };
    let files = l.files.iter().map(|f| rel(root, f)).collect();
    (keys, files, l.uncertainties.len())
}

#[test]
fn jest_and_vitest_do_not_follow_factories() {
    let root = fixture();
    // `defineProject(x)` of the workspace package adds `passWithNoTests`. The
    // default (jest, vitest): the package is not read and the call is `x`.
    assert_eq!(
        loaded(&root, "apps/legacy/jest.config.ts", false),
        (
            vec!["testMatch".to_string()],
            vec!["apps/legacy/jest.config.ts".to_string()],
            0
        )
    );
    // The opt-in is what makes the difference.
    assert_eq!(
        loaded(&root, "apps/legacy/jest.config.ts", true),
        (
            vec!["testMatch".to_string(), "passWithNoTests".to_string()],
            vec![
                "apps/legacy/jest.config.ts".to_string(),
                "tooling/playwright/src/config/project.ts".to_string()
            ],
            0
        )
    );

    for (fw, config) in [
        ("jest", "apps/legacy/jest.config.ts"),
        ("vitest", "apps/legacy/vitest.config.ts"),
    ] {
        let inputs = framework(fw).unwrap().load(&root.join(config));
        assert_eq!(
            uncertainties(&inputs),
            Vec::<&FrameworkUncertainty>::new(),
            "{fw}"
        );
        assert_eq!(files(&root, &inputs), [config], "{fw}");
        let tests: Vec<String> = inputs
            .projects
            .iter()
            .flat_map(|p| p.discovery.enumerate())
            .map(|t| rel(&root, &t))
            .collect();
        assert_eq!(tests, ["apps/legacy/src/sum.test.ts"], "{fw}");
    }
}

#[test]
fn workspace_reached_through_a_link_keeps_the_callers_paths() {
    // A workspace whose root is named through a symlink (`/tmp` on macOS, a linked
    // checkout): the factory's files are reported under the root as the caller
    // names it.
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("real");
    let write = |p: &str, s: &str| {
        let f = real.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, s).unwrap();
    };
    write("package.json", r#"{ "private": true }"#);
    write(
        "tooling/e2e/package.json",
        r#"{ "name": "@acme/e2e", "main": "./dist/index.js" }"#,
    );
    write(
        "tooling/e2e/src/index.ts",
        "export const base = (dir: string) => ({ testDir: './', testMatch: '*.e2e.ts' });\n",
    );
    write(
        "apps/a/playwright.config.ts",
        "import { base } from '@acme/e2e';\nexport default base(__dirname);\n",
    );
    write("apps/a/flows/pay.e2e.ts", "");
    write("apps/a/lib/pay.test.ts", "");
    std::fs::create_dir_all(real.join("apps/a/node_modules/@acme")).unwrap();
    turborepo_trace_core::os::symlink(
        "../../../../tooling/e2e",
        real.join("apps/a/node_modules/@acme/e2e"),
    )
    .unwrap();
    let alias = tmp.path().join("alias");
    turborepo_trace_core::os::symlink(&real, &alias).unwrap();

    let (inputs, _) = playwright::load_config(&alias.join("apps/a/playwright.config.ts"));
    assert_eq!(specs(&alias, &inputs), ["apps/a/flows/pay.e2e.ts"]);
    assert_eq!(
        files(&alias, &inputs),
        ["apps/a/playwright.config.ts", "tooling/e2e/src/index.ts"]
    );
    assert_eq!(uncertainties(&inputs), Vec::<&FrameworkUncertainty>::new());
}
