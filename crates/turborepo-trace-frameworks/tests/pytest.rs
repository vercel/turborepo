//! pytest adapter tests on `fixtures/frameworks/pytest-*`: rootdir and config
//! detection, config inputs, discovery, conftest chains, `--import-mode` roots
//! and plugins.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(feature = "python")]

use std::path::{Path, PathBuf};

use serde_json::json;
use turborepo_trace_frameworks::{
    Framework, InputRole, SimpleResolver, Target, framework,
    pytest::{self, ImportMode, Pytest},
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/frameworks")
        .join(name)
        .canonicalize()
        .expect("fixture exists")
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .into_owned()
}

fn entry_names(root: &Path, t: &turborepo_trace_frameworks::TestInputs) -> Vec<String> {
    t.entries
        .iter()
        .map(|e| match &e.target {
            Target::Path(p) => rel(root, p),
            Target::Module { request, .. } => format!("module:{request}"),
        })
        .collect()
}

#[test]
fn registered() {
    assert_eq!(framework("pytest").map(|f| f.id()), Some("pytest"));
}

#[test]
fn pyproject_config_and_discovery() {
    let root = fixture("pytest-basic");
    let test_a = root.join("tests/test_a.py");
    let config = Pytest.config_for(&test_a).unwrap();
    assert_eq!(config, root.join("pyproject.toml"));
    let cfg = pytest::load_config(&config);
    assert_eq!(cfg.rootdir, root);
    assert_eq!(cfg.import_mode, ImportMode::Importlib);
    assert_eq!(cfg.plugins, ["demo_plugin"]);
    assert_eq!(cfg.disabled_plugins, ["cacheprovider"]);
    assert_eq!(cfg.pythonpath, [root.join("src")]);
    assert_eq!(cfg.testpaths, ["tests"]);

    let inputs = Pytest.load(&config);
    let p = &inputs.projects[0];
    assert_eq!(p.global.config_files, [root.join("pyproject.toml")]);
    assert_eq!(
        p.global.probes,
        [root.join("pytest.ini"), root.join(".pytest.ini")]
    );
    assert_eq!(p.resolution.module_paths, [root.join("src")]);
    let tests: Vec<String> = p
        .discovery
        .enumerate()
        .iter()
        .map(|t| rel(&root, t))
        .collect();
    // build/, .hidden/ (norecursedirs) and --ignore=tests/skipme are excluded.
    assert_eq!(
        tests,
        [
            "tests/test_a.py",
            "tests/unit/check_c.py",
            "tests/unit/test_b.py"
        ]
    );

    let resolver = SimpleResolver::default();
    let ti = Pytest.test_inputs(p, &test_a, b"from demo import thing\n", &resolver);
    assert_eq!(
        entry_names(&root, &ti),
        [
            "conftest.py",
            "tests/conftest.py",
            "module:tests.fixtures.db"
        ]
    );
    let roles: Vec<InputRole> = ti.entries.iter().map(|e| e.role).collect();
    assert_eq!(
        roles,
        [InputRole::Conftest, InputRole::Conftest, InputRole::Plugin]
    );
    // `--import-mode=importlib`: only `pythonpath` is added for the test.
    assert_eq!(ti.import_roots.front, [root.join("src")]);
    assert!(ti.import_roots.back.is_empty());
    assert_eq!(ti.package.as_ref().map(|p| p.name.as_str()), Some("demo"));
    assert_eq!(ti.probes, [root.join("tests/__init__.py")]);
    assert_eq!(ti.memberships[0].dir, root.join("tests/__snapshots__"));
    assert_eq!(ti.memberships[0].pattern, "test_a*");

    let test_b = root.join("tests/unit/test_b.py");
    let src = std::fs::read(&test_b).unwrap();
    let ti = Pytest.test_inputs(p, &test_b, &src, &resolver);
    assert_eq!(
        entry_names(&root, &ti),
        [
            "conftest.py",
            "tests/conftest.py",
            "tests/unit/__init__.py",
            "module:tests.fixtures.db",
            "module:tests.fixtures.extra"
        ]
    );
    assert_eq!(
        ti.probes,
        [
            root.join("tests/unit/conftest.py"),
            root.join("tests/__init__.py")
        ]
    );
    // importlib mode: only pythonpath goes on sys.path.
    let (front, back) = pytest::test_import_roots(&cfg, &test_b);
    assert_eq!(front, [root.join("src")]);
    assert!(back.is_empty());
}

#[test]
fn rootdir_rules_and_confcutdir() {
    let root = fixture("pytest-rootdir");
    let test = root.join("pkg/sub/thing_spec.py");
    // setup.cfg without [tool:pytest] is not a config; pytest.ini wins over
    // tox.ini.
    let (rootdir, config) = pytest::rootdir_for(&test);
    assert_eq!(rootdir, root);
    assert_eq!(config, Some(root.join("pytest.ini")));
    let inputs = Pytest.load(&root.join("pytest.ini"));
    let p = &inputs.projects[0];
    assert!(p.global.probes.is_empty());
    let tests: Vec<String> = p
        .discovery
        .enumerate()
        .iter()
        .map(|t| rel(&root, t))
        .collect();
    assert_eq!(tests, ["pkg/sub/thing_spec.py"]);
    let ti = Pytest.test_inputs(p, &test, b"", &SimpleResolver::default());
    // confcutdir = pkg: the root conftest.py is not loaded.
    assert_eq!(entry_names(&root, &ti), ["pkg/conftest.py"]);
    assert_eq!(
        ti.probes,
        [
            root.join("pkg/sub/conftest.py"),
            root.join("pkg/sub/__init__.py")
        ]
    );
}

#[test]
fn setup_py_rootdir_prepend_mode() {
    let root = fixture("pytest-setuppy");
    let test = root.join("lib/tests/test_x.py");
    let config = Pytest.config_for(&test).unwrap();
    assert_eq!(config, root.join("setup.py"));
    let inputs = Pytest.load(&config);
    let p = &inputs.projects[0];
    assert_eq!(p.root_dir, root);
    assert!(p.global.config_files.is_empty());
    let cfg = pytest::load_config(&config);
    assert_eq!(cfg.import_mode, ImportMode::Prepend);
    let (front, back) = pytest::test_import_roots(&cfg, &test);
    assert_eq!(front, [root.join("lib/tests")]);
    assert!(back.is_empty());
    let ti = Pytest.test_inputs(p, &test, b"", &SimpleResolver::default());
    assert_eq!(
        ti.probes,
        [
            root.join("conftest.py"),
            root.join("lib/conftest.py"),
            root.join("lib/tests/conftest.py"),
            root.join("lib/tests/__init__.py"),
        ]
    );
}

#[test]
fn snapshot_of_inputs() {
    let root = fixture("pytest-basic");
    let inputs = Pytest.load(&root.join("pyproject.toml"));
    let p = &inputs.projects[0];
    let tests: Vec<_> = p
        .discovery
        .enumerate()
        .iter()
        .map(|t| {
            let src = std::fs::read(t).unwrap();
            Pytest.test_inputs(p, t, &src, &SimpleResolver::default())
        })
        .collect();
    let v = json!({ "inputs": inputs, "tests": tests });
    let s = serde_json::to_string_pretty(&v)
        .unwrap()
        .replace(&*root.to_string_lossy(), "<fixture>");
    insta::assert_snapshot!("pytest_basic", s);
}
