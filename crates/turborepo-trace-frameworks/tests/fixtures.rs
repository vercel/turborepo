//! Fixture tests: each fixture under `fixtures/frameworks/` is loaded, its
//! tests are enumerated, and the resulting `FrameworkInputs` plus per-test
//! inputs are snapshotted (paths relative to the fixture). Explicit assertions
//! pin the important behaviors.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use serde_json::json;
use turborepo_trace_frameworks::{
    EdgeKind, Framework, FrameworkInputs, InputRole, SimpleResolver, Target, TestInputs, framework,
    jest::Jest, vitest::Vitest,
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/frameworks")
        .join(name)
        .canonicalize()
        .expect("fixture exists")
}

struct Run {
    root: PathBuf,
    inputs: FrameworkInputs,
    tests: Vec<(String, TestInputs)>,
}

fn run(fw: &dyn Framework, name: &str, config: &str) -> Run {
    let root = fixture(name);
    let inputs = fw.load(&root.join(config));
    let mut tests = Vec::new();
    for p in &inputs.projects {
        for t in p.discovery.enumerate() {
            let src = std::fs::read(&t).unwrap();
            tests.push((
                p.name.clone(),
                fw.test_inputs(p, &t, &src, &SimpleResolver::default()),
            ));
        }
    }
    Run {
        root,
        inputs,
        tests,
    }
}

impl Run {
    fn json(&self) -> String {
        let tests: Vec<_> = self
            .tests
            .iter()
            .map(|(p, t)| json!({ "project": p, "inputs": t }))
            .collect();
        let v = json!({ "inputs": self.inputs, "tests": tests });
        let s = serde_json::to_string_pretty(&v).unwrap();
        s.replace(&*self.root.to_string_lossy(), "<fixture>")
    }

    fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.root)
            .unwrap_or(p)
            .to_string_lossy()
            .into_owned()
    }

    fn test(&self, rel: &str) -> &TestInputs {
        &self
            .tests
            .iter()
            .find(|(_, t)| self.rel(&t.test) == rel)
            .unwrap_or_else(|| panic!("no test {rel}"))
            .1
    }

    fn test_names(&self) -> Vec<String> {
        self.tests.iter().map(|(_, t)| self.rel(&t.test)).collect()
    }
}

#[test]
fn jest_basic() {
    let r = run(&Jest, "jest-basic", "jest.config.js");
    insta::assert_snapshot!("jest_basic", r.json());
    let p = &r.inputs.projects[0];
    assert_eq!(p.name, "basic");
    // Discovery: legacy is ignored, non-tests are not listed.
    assert_eq!(
        r.test_names(),
        ["src/__tests__/db.test.ts", "src/components/button.spec.tsx"]
    );
    // Mapper with regex groups and <rootDir>, then the helper's mappers, in order.
    let hit = p.resolution.apply("@app/lib/x").unwrap();
    assert_eq!(
        hit.requests,
        [r.root.join("src/lib/x").to_string_lossy().into_owned()]
    );
    let hit = p.resolution.apply("./a/b.js").unwrap();
    assert_eq!(hit.requests, ["./a/b"]);
    let css = p.resolution.apply("./x.css").unwrap();
    assert_eq!(css.requests.len(), 2);
    // Config files: the config and its helper, both traced as JS.
    let cfgs: Vec<String> = p.global.config_files.iter().map(|f| r.rel(f)).collect();
    assert_eq!(cfgs, ["jest.config.js", "jest.base.js"]);
    // Entries.
    let roles: Vec<InputRole> = p.global.entries.iter().map(|e| e.role).collect();
    for want in [
        InputRole::SetupFile,
        InputRole::SetupFileAfterEnv,
        InputRole::GlobalSetup,
        InputRole::TestEnvironment,
        InputRole::Transform,
    ] {
        assert!(roles.contains(&want), "{want:?} in {roles:?}");
    }
    // The env-dependent setup file is kept (over-approximation) and reported.
    assert!(
        p.global
            .entries
            .iter()
            .any(|e| matches!(&e.target, Target::Path(t) if t.ends_with("ci-setup.ts")))
    );
    // Per test: manual mocks (local adjacent, root package, haste), automock,
    // snapshot.
    let t = r.test("src/__tests__/db.test.ts");
    let edges: Vec<(EdgeKind, String)> = t.edges.iter().map(|e| (e.kind, r.rel(&e.to))).collect();
    assert!(
        edges.contains(&(EdgeKind::ManualMock, "src/lib/__mocks__/db.ts".into())),
        "{edges:?}"
    );
    assert!(
        edges.contains(&(EdgeKind::ManualMock, "__mocks__/axios.js".into())),
        "{edges:?}"
    );
    assert!(edges.contains(&(
        EdgeKind::Snapshot,
        "src/__tests__/__snapshots__/db.test.ts.snap".into()
    )));
    assert!(
        t.probes
            .iter()
            .any(|p| p.ends_with("src/lib/__mocks__/cache.ts"))
    );
    assert!(
        t.probes
            .iter()
            .any(|p| p.ends_with("__mocks__/left-pad.js"))
    );
    assert!(t.entries.iter().any(|e| e.role == InputRole::Automocked
        && matches!(&e.target, Target::Module { request, .. } if request == "../lib/cache")));
    assert!(t.entries.iter().any(|e| e.role == InputRole::RequireActual));
    // Mapped mock: `@app/lib/x` resolves through the mapper; its adjacent mock is
    // probed.
    assert!(
        t.probes
            .iter()
            .any(|p| p.ends_with("src/lib/__mocks__/x.ts"))
    );
    // Docblock environment.
    let b = r.test("src/components/button.spec.tsx");
    assert!(b.entries.iter().any(|e| e.role == InputRole::TestEnvironment
        && matches!(&e.target, Target::Module { request, .. } if request == "jest-environment-node")));
    assert!(
        b.memberships
            .iter()
            .any(|m| m.pattern == "button.spec.tsx.snap")
    );
    // config_for / discover.
    assert_eq!(
        Jest.config_for(&r.root.join("src/lib/db.ts")),
        Some(r.root.join("jest.config.js"))
    );
    assert_eq!(Jest.discover(&r.root), [r.root.join("jest.config.js")]);
}

#[test]
fn jest_projects() {
    let r = run(&Jest, "jest-projects", "jest.config.js");
    insta::assert_snapshot!("jest_projects", r.json());
    let names: Vec<&str> = r.inputs.projects.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names.len(), 4, "{names:?}");
    assert!(names.contains(&"a") && names.contains(&"b") && names.contains(&"tools"));
    assert_eq!(
        r.test_names(),
        [
            "packages/a/src/a.test.ts",
            "packages/b/lib/b.spec.js",
            "packages/c/test/c.test.js",
            "tools/lint.check.js"
        ]
    );
    let a = r.inputs.projects.iter().find(|p| p.name == "a").unwrap();
    // The root config is an input of every project.
    assert_eq!(r.rel(&a.global.config_files[0]), "jest.config.js");
    assert_eq!(
        a.resolution.apply("~/a").unwrap().requests,
        [r.root
            .join("packages/a/src/a")
            .to_string_lossy()
            .into_owned()]
    );
    let b = r.inputs.projects.iter().find(|p| p.name == "b").unwrap();
    assert!(
        b.global
            .entries
            .iter()
            .any(|e| e.role == InputRole::TestEnvironment
                && matches!(&e.target, Target::Path(p) if p.ends_with("packages/b/custom-env.js")))
    );
    assert!(
        b.global
            .config_files
            .iter()
            .any(|f| f.ends_with("packages/b/package.json"))
    );
    let tools = r
        .inputs
        .projects
        .iter()
        .find(|p| p.name == "tools")
        .unwrap();
    assert_eq!(tools.root_dir, r.root.join("tools"));
    let found = r
        .inputs
        .project_for(&r.root.join("packages/b/lib/b.spec.js"))
        .unwrap();
    assert_eq!(found.name, "b");
}

#[test]
fn jest_rootdir_preset_resolver() {
    let r = run(&Jest, "jest-rootdir", "config/jest.config.mjs");
    insta::assert_snapshot!("jest_rootdir", r.json());
    let p = &r.inputs.projects[0];
    assert_eq!(p.root_dir, r.root);
    assert_eq!(p.resolution.roots, [r.root.join("src")]);
    assert_eq!(p.resolution.module_paths, [r.root.join("shared")]);
    assert_eq!(
        p.resolution.module_directories,
        [
            "node_modules".to_string(),
            r.root.to_string_lossy().into_owned()
        ]
    );
    // testRegex, testPathIgnorePatterns and modulePathIgnorePatterns with
    // <rootDir>.
    assert_eq!(r.test_names(), ["src/feature/f.unit.ts"]);
    // Local preset merged: its setup file and mapper.
    assert!(
        p.global
            .entries
            .iter()
            .any(|e| matches!(&e.target, Target::Path(t) if t.ends_with("config/preset-setup.js")))
    );
    assert!(p.resolution.apply("#shared/s").is_some());
    assert!(
        p.global
            .config_files
            .iter()
            .any(|f| f.ends_with("config/preset.json"))
    );
    // Custom snapshot resolver: an entry, a broad membership and an uncertainty.
    let t = r.test("src/feature/f.unit.ts");
    assert!(t.memberships.iter().any(|m| m.pattern == "**/*.snap"));
    assert!(!t.uncertainties.is_empty());
    assert!(
        p.global
            .entries
            .iter()
            .any(|e| e.role == InputRole::SnapshotResolver)
    );
}

#[test]
fn vitest_alias_merge_mocks() {
    let r = run(&Vitest, "vitest-alias", "vitest.config.ts");
    insta::assert_snapshot!("vitest_alias", r.json());
    let p = &r.inputs.projects[0];
    assert_eq!(r.test_names(), ["src/app.test.ts"]);
    // test.alias first, then resolve.alias from the merged vite config (regex and
    // prefix).
    let sources: Vec<&str> = p.resolution.aliases.iter().map(|a| a.source).collect();
    assert_eq!(sources, ["test.alias", "resolve.alias", "resolve.alias"]);
    assert_eq!(
        p.resolution.apply("~icons/home").unwrap().requests,
        [r.root
            .join("src/icons/home.svg")
            .to_string_lossy()
            .into_owned()]
    );
    assert_eq!(
        p.resolution.apply("@lib/api").unwrap().requests,
        [r.root.join("src/lib/api").to_string_lossy().into_owned()]
    );
    let cfgs: Vec<String> = p.global.config_files.iter().map(|f| r.rel(f)).collect();
    assert_eq!(cfgs, ["vitest.config.ts", "vite.config.ts"]);
    let t = r.test("src/app.test.ts");
    let edges: Vec<String> = t.edges.iter().map(|e| r.rel(&e.to)).collect();
    assert!(
        edges.contains(&"src/lib/__mocks__/api.ts".to_string()),
        "{edges:?}"
    );
    assert!(
        edges.contains(&"__mocks__/lodash.ts".to_string()),
        "{edges:?}"
    );
    assert!(edges.contains(&"src/__snapshots__/app.test.ts.snap".to_string()));
    assert!(
        t.probes
            .iter()
            .any(|p| p.ends_with("src/__mocks__/missing.ts"))
    );
    assert!(
        t.entries
            .iter()
            .any(|e| e.role == InputRole::TestEnvironment
                && matches!(&e.target, Target::Module { request, .. } if request == "happy-dom"))
    );
    assert!(t.entries.iter().any(|e| e.role == InputRole::RequireActual));
    assert!(p.uncertainties.is_empty(), "{:?}", p.uncertainties);
}

#[test]
fn vitest_workspace() {
    let r = run(&Vitest, "vitest-workspace", "vitest.workspace.ts");
    insta::assert_snapshot!("vitest_workspace", r.json());
    let mut names: Vec<&str> = r.inputs.projects.iter().map(|p| p.name.as_str()).collect();
    names.sort();
    assert_eq!(names.len(), 3, "{names:?}");
    assert!(names.contains(&"web") && names.contains(&"scripts"));
    assert_eq!(
        r.test_names(),
        [
            "packages/core/src/core.spec.ts",
            "packages/web/src/page.test.ts",
            "scripts/build.test.ts"
        ]
    );
    let scripts = r
        .inputs
        .projects
        .iter()
        .find(|p| p.name == "scripts")
        .unwrap();
    // `extends` merged the shared config: setup file and alias.
    assert!(
        scripts
            .global
            .entries
            .iter()
            .any(|e| matches!(&e.target, Target::Path(t) if t.ends_with("vitest.setup.ts")))
    );
    assert!(scripts.resolution.apply("#root/x").is_some());
    let cfgs: Vec<String> = scripts
        .global
        .config_files
        .iter()
        .map(|f| r.rel(f))
        .collect();
    assert_eq!(cfgs, ["vitest.workspace.ts", "vitest.shared.ts"]);
    let web = r.inputs.projects.iter().find(|p| p.name == "web").unwrap();
    assert!(
        web.global
            .config_files
            .iter()
            .any(|f| f.ends_with("vitest.workspace.ts"))
    );
    assert!(
        web.global.entries.iter().any(
            |e| matches!(&e.target, Target::Path(t) if t.ends_with("packages/web/src/setup.ts"))
        )
    );
}

#[test]
fn registry() {
    assert!(framework("jest").is_some());
    assert!(framework("vitest").is_some());
    assert_eq!(framework("pytest").is_some(), cfg!(feature = "python"));
    let root = fixture("vitest-workspace");
    let found = Vitest.discover(&root);
    assert_eq!(found[0], root.join("vitest.workspace.ts"));
    assert!(Vitest.is_config_file(&root.join("packages/web/vitest.config.mts")));
}

/// M1: `export default async () => ({ ...(await nextJest({ dir })(custom)())
/// })` (the shape of `apps/web/jest.config.ts` in a large monorepo) used to
/// yield zero tests.
#[test]
fn jest_next_async_wrapper() {
    let r = run(&Jest, "jest-next-async", "jest.config.ts");
    assert_eq!(r.test_names(), ["src/a.test.ts"]);
    let p = &r.inputs.projects[0];
    assert!(
        p.global
            .entries
            .iter()
            .any(|e| matches!(&e.target, Target::Path(t) if t.ends_with("test/setup.ts"))),
        "setupFilesAfterEnv from the custom config"
    );
    assert!(
        !p.uncertainties
            .iter()
            .chain(&r.inputs.uncertainties)
            .any(|u| u.field.as_deref().is_some_and(|f| f.contains("testMatch"))),
        "discovery is fully known: {:?}",
        p.uncertainties
    );
    // The config rewrites next's `transform` entry
    // (`Object.fromEntries(Object.entries(config.transform ?? {}).map(...))`) to
    // swap the SWC transformer for a file of its own. The field used to be
    // unknown, which made every test of the project uncertain. It is known now,
    // and the project's transformer is an input of every test (as is next's:
    // both branches of the callback count).
    let all: Vec<_> = p
        .uncertainties
        .iter()
        .chain(&r.inputs.uncertainties)
        .collect();
    assert!(all.is_empty(), "no uncertainty: {all:?}");
    let transforms: Vec<String> = p
        .global
        .entries
        .iter()
        .filter(|e| e.role == InputRole::Transform)
        .map(|e| format!("{:?}", e.target))
        .collect();
    assert!(
        transforms
            .iter()
            .any(|t| t.contains("test/compiler-transformer.cjs")),
        "{transforms:?}"
    );
    assert!(
        transforms
            .iter()
            .any(|t| t.contains("next/dist/build/swc/jest-transformer")),
        "{transforms:?}"
    );
}

/// Never silently zero tests: an unreadable `testMatch` falls back to jest's
/// default discovery under `rootDir`, with an uncertainty.
#[test]
fn jest_unknown_test_match_falls_back_to_defaults() {
    let r = run(&Jest, "jest-unknown-testmatch", "jest.config.js");
    assert_eq!(r.test_names(), ["lib/__tests__/b.js", "src/a.test.js"]);
    let p = &r.inputs.projects[0];
    assert!(
        p.uncertainties
            .iter()
            .any(|u| u.detail.contains("default testMatch")),
        "{:?}",
        p.uncertainties
    );
}

/// A discovery that matches no file falls back too (with an uncertainty) when
/// the config was not read exactly: here an unknown spread may replace
/// `testMatch`.
#[test]
fn jest_empty_discovery_falls_back_to_defaults() {
    let r = run(&Jest, "jest-matches-nothing-open", "jest.config.js");
    assert_eq!(r.test_names(), ["spec/s.test.js"]);
    let p = &r.inputs.projects[0];
    assert!(
        p.uncertainties
            .iter()
            .any(|u| u.detail.contains("match no file")),
        "{:?}",
        p.uncertainties
    );
}

/// A discovery that was read exactly and matches no file is a fact: the project
/// has no test, and nothing is uncertain. The fallback used to turn the files
/// of another runner in the same directory (Playwright `*.spec.ts` next to a
/// jest config for `*.test.ts`) into jest tests, each with an uncertainty.
#[test]
fn jest_exact_discovery_that_matches_nothing_has_no_tests() {
    for (fixture, config) in [
        ("jest-matches-nothing", "jest.config.js"),
        ("jest-playwright-specs", "playwright/jest.config.ts"),
    ] {
        let r = run(&Jest, fixture, config);
        assert!(r.test_names().is_empty(), "{fixture}: {:?}", r.test_names());
        let p = &r.inputs.projects[0];
        let all: Vec<_> = p
            .uncertainties
            .iter()
            .chain(&r.inputs.uncertainties)
            .collect();
        assert!(all.is_empty(), "{fixture}: {all:?}");
    }
}

/// `testMatch` with plain groups: `*.(test|spec).[jt]s?(x)` is `test` or `spec`
/// in picomatch. The translator used to take the parentheses literally, the
/// pattern matched nothing, and discovery fell back to the default patterns
/// with an uncertainty (which also lost the negated pattern).
#[test]
fn jest_test_match_with_plain_groups() {
    let r = run(&Jest, "jest-testmatch-groups", "jest.config.js");
    assert_eq!(r.test_names(), ["src/__tests__/b.spec.js", "src/a.test.ts"]);
    let p = &r.inputs.projects[0];
    assert!(p.uncertainties.is_empty(), "{:?}", p.uncertainties);
}

/// `testMatch: ['**/*.test.ts|**/*.spec.ts']`: a `|` outside a group goes into
/// picomatch's regex as is, and jest runs `src/a.test.ts`. The translator reads
/// the `|` literally, so the pattern matches no file here. It is not an exact
/// reading: discovery falls back to the default patterns, with an uncertainty,
/// instead of concluding that the project has no tests.
#[test]
fn jest_test_match_with_a_top_level_pipe_is_not_exact() {
    let r = run(&Jest, "jest-testmatch-pipe", "jest.config.js");
    assert_eq!(r.test_names(), ["src/a.test.ts"]);
    let p = &r.inputs.projects[0];
    assert!(
        p.uncertainties
            .iter()
            .any(|u| u.detail.contains("default testMatch")),
        "{:?}",
        p.uncertainties
    );
}

/// The fallback that stays: a `testMatch` with one entry the evaluator cannot
/// read, whose readable entry matches no file, and a preset that is not
/// installed while `roots` holds no test. Neither discovery was read exactly,
/// so neither project is believed to be empty.
#[test]
fn jest_inexact_discovery_that_matches_nothing_falls_back() {
    for fixture in ["jest-testmatch-partly-unknown", "jest-preset-missing"] {
        let r = run(&Jest, fixture, "jest.config.js");
        assert_eq!(r.test_names(), ["spec/s.test.js"], "{fixture}");
        let p = &r.inputs.projects[0];
        assert!(
            p.uncertainties
                .iter()
                .any(|u| u.detail.contains("default testMatch")),
            "{fixture}: {:?}",
            p.uncertainties
        );
    }
}

/// P11: where `process.cwd()` points for a project's tests: the config's
/// directory, then `rootDir` when it differs.
#[test]
fn jest_test_cwds() {
    let r = run(&Jest, "jest-rootdir", "config/jest.config.mjs");
    let cwds: Vec<String> = r.inputs.projects[0]
        .test_cwds()
        .iter()
        .map(|p| r.rel(p))
        .collect();
    assert_eq!(cwds, ["config", ""]);
    let r = run(&Jest, "jest-basic", "jest.config.js");
    assert_eq!(
        r.inputs.projects[0].test_cwds(),
        std::slice::from_ref(&r.root)
    );
}

/// `plugins: [react()]` (`@vitejs/plugin-react`: a JSX transform that redirects
/// no import) is not uncertain. Every vitest config with plugins used to carry
/// a `conditional_resolution` uncertainty, which selects all tests of the
/// project on every change. A plugin whose effect is not known still does, and
/// is named.
#[test]
fn vitest_known_plugins_are_not_uncertain() {
    let r = run(&Vitest, "vitest-plugins", "vitest.config.ts");
    assert_eq!(r.test_names(), ["src/a.test.tsx"]);
    let all: Vec<_> = r.inputs.projects[0]
        .uncertainties
        .iter()
        .chain(&r.inputs.uncertainties)
        .collect();
    assert!(all.is_empty(), "{all:?}");

    let r = run(&Vitest, "vitest-plugins-unknown", "vitest.config.ts");
    assert_eq!(r.test_names(), ["src/b.test.ts"]);
    let plugins: Vec<&str> = r.inputs.projects[0]
        .uncertainties
        .iter()
        .chain(&r.inputs.uncertainties)
        .filter(|u| u.field.as_deref() == Some("plugins"))
        .map(|u| u.detail.as_str())
        .collect();
    assert_eq!(plugins.len(), 1, "{plugins:?}");
    assert!(
        plugins[0].contains("@acme/shaders/loader-vite") && !plugins[0].contains("plugin-react"),
        "{plugins:?}"
    );
}

/// A vite alias whose `find` is `new RegExp(...)` over a template string, built
/// from a list with `.map(x => x.replace(/\./g, '\\.')).join('|')` inside a
/// negative lookahead, with the replacement `${projectRoot}$1` (`projectRoot`:
/// `fileURLToPath(new URL('.', import.meta.url))`, which ends with `/`). The
/// whole `resolve.alias` field used to be unknown (`computed_specifier` on
/// every test of the project). It is evaluated exactly: the lookahead excludes
/// the generated files, every other `#/…` request maps into the project.
#[test]
fn vitest_regex_alias_from_new_regexp() {
    let r = run(&Vitest, "vitest-alias-regex", "vitest.config.ts");
    assert_eq!(r.test_names(), ["lib/util.test.ts"]);
    let p = &r.inputs.projects[0];
    let all: Vec<_> = p
        .uncertainties
        .iter()
        .chain(&r.inputs.uncertainties)
        .collect();
    assert!(all.is_empty(), "{all:?}");
    let abs = |rel: &str| r.root.join(rel).to_string_lossy().into_owned();
    assert_eq!(
        p.resolution.apply("#/lib/util").unwrap().requests,
        [abs("lib/util")]
    );
    for generated in [
        "#/app/routes.mjs",
        "#/lib/valid-paths.mjs",
        "#/lib/flag-codes.mjs",
    ] {
        assert!(p.resolution.apply(generated).is_none(), "{generated}");
    }
    // Only the exact names are excluded.
    assert_eq!(
        p.resolution.apply("#/app/routes.mjs.map").unwrap().requests,
        [abs("app/routes.mjs.map")]
    );
    assert_eq!(
        p.resolution.apply("server-only").unwrap().requests,
        [abs("test/server-only-stub.ts")]
    );
    assert_eq!(
        p.resolution.apply("next/font/google").unwrap().requests,
        [abs("test/font-stub.ts")]
    );
    assert!(p.resolution.apply("next/font/other").is_none());
}

/// Alias targets written as `path.resolve(__dirname, './src')` are values the
/// config hands to the runner. The adapter reports them
/// ([`turborepo_trace_frameworks::ConfigValues`]) so that the graph does not
/// also treat them as directories the config module reads, and it models
/// an alias `customResolver` that is `require.resolve` of its argument.
#[test]
fn vitest_config_values_are_reported() {
    let r = run(&Vitest, "vitest-config-values", "vitest.config.ts");
    assert_eq!(r.test_names(), ["src/lib/a.test.ts", "src/lib/b.test.ts"]);
    let p = &r.inputs.projects[0];
    let all: Vec<_> = p
        .uncertainties
        .iter()
        .chain(&r.inputs.uncertainties)
        .collect();
    assert!(all.is_empty(), "{all:?}");
    let v = p.global.values.as_ref().expect("config values");
    assert_eq!(r.rel(&v.file), "vitest.config.ts");
    let paths: Vec<String> = v.paths.iter().map(|p| r.rel(p)).collect();
    assert_eq!(
        paths,
        [
            "src",
            "src/app/[variant]",
            "src/app/[variant]/(site)$1",
            "src/app.config.ts",
            "test/mocks/font.ts",
            "vendor",
        ]
    );
    // The `customResolver` function is modelled: one byte range, the arrow
    // function.
    let src = std::fs::read_to_string(r.root.join("vitest.config.ts")).unwrap();
    let modelled: Vec<&str> = v
        .modelled
        .iter()
        .map(|&(a, b)| &src[a as usize..b as usize])
        .collect();
    assert_eq!(modelled.len(), 1, "{modelled:?}");
    assert!(
        modelled[0].starts_with("(specifier: string)")
            && modelled[0].contains("require.resolve(specifier)"),
        "{modelled:?}"
    );
    // The aliases themselves.
    let abs = |rel: &str| r.root.join(rel).to_string_lossy().into_owned();
    assert_eq!(
        p.resolution.apply("@/lib/a").unwrap().requests,
        [abs("src/lib/a")]
    );
    assert_eq!(
        p.resolution.apply("@/app/(site)/page").unwrap().requests,
        [abs("src/app/[variant]/(site)/page")]
    );
    assert_eq!(
        p.resolution.apply("@acme/content/posts").unwrap().requests,
        ["@acme/content/posts"]
    );
}

/// `plugins: [react(), wgslVitePlugin({ minify: true })]`: the second plugin is
/// a loader for `.wgsl` modules. The project is not uncertain as a whole; the
/// adapter reports the loader with its suffixes, and the graph makes the
/// importers of such modules uncertain.
#[test]
fn vitest_loader_plugin_is_scoped() {
    let r = run(&Vitest, "vitest-plugins-loader", "vitest.config.ts");
    assert_eq!(r.test_names(), ["src/plain.test.ts", "src/render.test.ts"]);
    let p = &r.inputs.projects[0];
    let all: Vec<_> = p
        .uncertainties
        .iter()
        .chain(&r.inputs.uncertainties)
        .collect();
    assert!(all.is_empty(), "{all:?}");
    let loaders: Vec<(&str, &[String])> = p
        .resolution
        .loaders
        .iter()
        .map(|l| (l.plugin.as_str(), l.suffixes.as_slice()))
        .collect();
    assert_eq!(
        loaders,
        [("@vgpu/wgsl/loader-vite", &[".wgsl".to_string()][..])]
    );
}

/// A `moduleNameMapper` target built with `require('fs').realpathSync(...)`:
/// `path.resolve(realpathSync(<the link of a dependency>),
/// '../shared/dist/index.js')` names a sibling of the dependency in the package
/// manager's store. The whole mapper used to be unknown (`computed_specifier`
/// on every test). The real path of a known path is known: symlinks are
/// resolved through the file system; a path that does not exist (no
/// install) stays as written.
#[test]
fn jest_mapper_target_through_realpath() {
    // Without an install.
    let r = run(&Jest, "jest-realpath-mapper", "jest.config.js");
    assert_eq!(r.test_names(), ["src/chat.test.js"]);
    let p = &r.inputs.projects[0];
    let all: Vec<_> = p
        .uncertainties
        .iter()
        .chain(&r.inputs.uncertainties)
        .collect();
    assert!(all.is_empty(), "{all:?}");
    assert_eq!(
        p.resolution.apply("@scope/shared").unwrap().requests,
        [r.root
            .join("node_modules/@scope/shared/dist/index.js")
            .to_string_lossy()
            .into_owned()]
    );
    // The shared config's mapper is still there, after the package's own entry.
    assert!(p.resolution.apply("@test/x").is_some());

    // With a pnpm-style install: the link's real path is in the store.
    #[cfg(unix)]
    {
        let t = std::env::temp_dir().join(format!("tt-realpath-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let pkg = t.join("pkg");
        std::fs::create_dir_all(pkg.join("node_modules/@scope")).unwrap();
        for f in ["jest.config.js", "jest.base.js", "jest-setup.ts"] {
            std::fs::copy(r.root.join(f), pkg.join(f)).unwrap();
        }
        let store = t.join("node_modules/.pnpm/@scope+x@1.0.0/node_modules/@scope");
        std::fs::create_dir_all(store.join("x")).unwrap();
        std::fs::create_dir_all(store.join("shared/dist")).unwrap();
        std::fs::write(store.join("shared/dist/index.js"), "exports.chat = 1;\n").unwrap();
        turborepo_trace_core::os::symlink(store.join("x"), pkg.join("node_modules/@scope/x"))
            .unwrap();
        let inputs = Jest.load(&pkg.join("jest.config.js"));
        let want = store
            .join("shared/dist/index.js")
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let got = inputs.projects[0]
            .resolution
            .apply("@scope/shared")
            .unwrap()
            .requests;
        std::fs::remove_dir_all(&t).unwrap();
        assert_eq!(got, [want]);
    }
}

/// A custom jest `resolver` module that is the default resolver with one more
/// export condition for one request. `resolver` used to make every test of the
/// project uncertain ("resolution may differ"). The adapter reads the resolver
/// module: the only thing that differs is which file of that request's package
/// is picked, so it reports the request
/// instead (`ResolutionOverrides::condition_requests`). The resolver stays an
/// input.
#[test]
fn jest_resolver_that_only_changes_conditions() {
    let r = run(&Jest, "jest-resolver-conditions", "jest.config.ts");
    assert_eq!(r.test_names(), ["lib/util.test.ts"]);
    let p = &r.inputs.projects[0];
    let all: Vec<_> = p
        .uncertainties
        .iter()
        .chain(&r.inputs.uncertainties)
        .collect();
    assert!(all.is_empty(), "{all:?}");
    assert_eq!(p.resolution.condition_requests, ["tree-walker"]);
    assert!(
        p.global
            .entries
            .iter()
            .any(|e| e.role == InputRole::Resolver
                && matches!(&e.target, Target::Path(t) if t.ends_with("jest.resolver.cjs"))),
        "{:?}",
        p.global.entries
    );
}
