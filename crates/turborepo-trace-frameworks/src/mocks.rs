//! Mock-aware loading: which modules a jest or vitest file replaces with a mock
//! that does not load them, and what undoes a replacement.
//!
//! A replacement holds for a test when all of this is true (measured with the
//! runtime mock oracle):
//!
//! - the call is `jest.mock` / `vi.mock` as a statement at the top level of the
//!   test file or of a setup file of its project: the runner's transform hoists
//!   it above the file's imports. A call in a hook, a block or a function is
//!   not hoisted by jest (the imports load the real module first), `doMock` /
//!   `setMock` never are, and a call in a helper file only replaces what is
//!   loaded after the helper;
//! - the request is a literal that resolves, through the project's aliases, to
//!   a first-party file;
//! - the second argument is an inline function without parameters (a factory
//!   whose value is the module), or there is none and a manual mock stands next
//!   to the module (`__mocks__/<file>`): then the test loads the manual mock
//!   instead. Without either the runner loads the real module to build an
//!   automock; with a parameter the factory gets `importOriginal`; `{ spy: true
//!   }` loads the real module;
//! - nothing loads the real module after all: `requireActual` / `importActual`
//!   / `createMockFromModule`, `unmock` / `dontMock` / `doUnmock`, a later mock
//!   without a factory (vitest re-registers an automock). Each of these names
//!   its module, which is then [`TestInputs::loaded_real`]; when the module is
//!   computed or the API is used in a way the scan cannot read, the file has a
//!   [`MockHazard`] and no replacement of a test that loads it counts.
//!
//! `resetModules` and `isolateModules` clear the module registry but keep the
//! registered mocks: they undo nothing.
//!
//! # What a mock loads, and when
//!
//! A mock call without a factory loads nothing. The runner loads the manual
//! mock, or the real module to build an automock from, when the module is first
//! requested after the call; a factory runs then too. So a file that is not a
//! test and registers such a mock (a helper, a file a setup file imports) does
//! not depend on the manual mock by itself: a test does that loads the file
//! *and* requests the module. [`TestInputs::lazy_mocks`] keeps the pair, the
//! graph layer adds an edge from the module to the mock for it, and
//! `turborepo_trace_graph::barrel::mocks` checks per test that it loads the
//! file. The source of an automock needs no edge at all: a test that requests
//! the module depends on it anyway. Only what loads when the helper's code runs
//! (`requireActual`, `requireMock`, a mock jest finds by name) is a dependency
//! of the helper itself ([`TestInputs::loaded_eager`]). `requireActual` in the
//! factory of a mock of the same module (a partial mock) is not: the factory
//! runs when the module is requested.
//!
//! Project-wide conditions are [`crate::MockPolicy::unreliable`]. What the
//! graph layer does with all this is in `turborepo_trace_graph::mocks`.

use std::path::{Path, PathBuf};

use turborepo_trace_core::eval::Value;
pub use turborepo_trace_lang_js::mocks::MockScan;
use turborepo_trace_lang_js::{
    extract::config::ConfigVal,
    mocks::{MockArg, MockCall, MockRole},
};

use crate::{
    common::{MockCx, ModuleResolver, is_path_request},
    fsx::{self, PathFsx},
    inputs::{LazyMock, MockHazard, Project, Replaced, TestInputs},
};

/// What a request stands for.
#[derive(Clone)]
enum Module {
    /// A first-party file.
    File(PathBuf),
    /// One of several first-party files (a request with alternatives).
    Many(Vec<PathBuf>),
    /// An installed package, a package that is not installed (the tracer
    /// answers with the manifest that declares it: a manifest is not a
    /// module, and a mock of the package does not make the test independent
    /// of the declared version), or a bare request that does not resolve
    /// (no importer has an edge through it either).
    Package,
    /// A path request that does not resolve.
    Unknown,
}

struct Cx<'a> {
    project: Option<&'a Project>,
    resolver: &'a dyn ModuleResolver,
    dir: &'a Path,
    /// What the requests of this file stand for (a file names the same module
    /// in several calls: a mock, `requireActual` in its factory, an
    /// unmock).
    modules: std::cell::RefCell<Vec<(String, Module)>>,
}

impl Cx<'_> {
    /// Whether `request` names its file by path ([`Replaced::by_path`]).
    fn by_path(&self, request: &str) -> bool {
        is_path_request(request)
            || self
                .project
                .is_some_and(|p| p.resolution.apply(request).is_some())
    }

    fn module(&self, request: &str) -> Module {
        if let Some((_, m)) = self.modules.borrow().iter().find(|(r, _)| r == request) {
            return m.clone();
        }
        let m = self.resolve(request);
        self.modules
            .borrow_mut()
            .push((request.to_owned(), m.clone()));
        m
    }

    fn resolve(&self, request: &str) -> Module {
        // The first matching alias decides, as in the runner; its replacements are
        // tried in order.
        let hit = self.project.and_then(|p| p.resolution.apply(request));
        let requests: Vec<&str> = match &hit {
            Some(h) => h.requests.iter().map(String::as_str).collect(),
            None => vec![request],
        };
        let files = requests
            .iter()
            .map(|r| self.resolver.resolve_all(r, self.dir))
            .find(|f| !f.is_empty())
            .unwrap_or_default();
        if files.is_empty() {
            return if hit.is_some() || is_path_request(request) {
                Module::Unknown
            } else {
                Module::Package
            };
        }
        let mut modules: Vec<PathBuf> = files
            .iter()
            .filter(|p| {
                p.file_name().is_some_and(|n| n != "package.json") && !fsx::in_node_modules(p)
            })
            .map(|p| fsx::normalize(p))
            .collect();
        modules.sort();
        modules.dedup();
        match modules.len() {
            0 => Module::Package,
            1 => Module::File(modules.remove(0)),
            _ => Module::Many(modules),
        }
    }
}

/// The manual mock next to `module` (`<dir>/__mocks__/<file name>`), if it
/// exists.
fn manual_mock(module: &Path) -> Option<PathBuf> {
    let p = module.parent()?.join("__mocks__").join(module.file_name()?);
    p.fx_is_file().then(|| fsx::normalize(&p))
}

/// Whether jest loads `file` as an ES module: `.mjs` / `.mts`, or `.js` in a
/// package with `"type": "module"`. `jest.mock` does not replace what such a
/// file imports statically.
fn jest_esm(file: &Path, root: &Path) -> bool {
    match file.extension().and_then(|e| e.to_str()) {
        Some("mjs" | "mts") => true,
        Some("js") => {
            let mut dir = file.parent();
            while let Some(d) = dir {
                if let Ok(text) = fsx::read_to_string(d.join("package.json")) {
                    return serde_json::from_str::<serde_json::Value>(&text)
                        .ok()
                        .and_then(|v| {
                            v.get("type")
                                .and_then(|t| t.as_str())
                                .map(|t| t == "module")
                        })
                        .unwrap_or(false);
                }
                if d == root {
                    break;
                }
                dir = d.parent();
            }
            false
        }
        _ => false,
    }
}

/// Transformer packages whose output has `jest.mock` calls hoisted above the
/// imports: `babel-jest` (always adds `babel-preset-jest`), `ts-jest` (its
/// hoisting transformer), `@swc/jest` (always sets the SWC option). A
/// transformer that only wraps a compiler (esbuild, a bare `@swc/core` call)
/// does not hoist: the imports then load the real module before the mock is
/// registered.
const HOISTING_TRANSFORMERS: &[&str] = &["babel-jest", "ts-jest", "@swc/jest"];

/// The transformer `next/jest` configures (SWC with the jest option).
const NEXT_JEST_TRANSFORMER: &str = "next/dist/build/swc/jest-transformer";

/// File names a transform of test code applies to.
const CODE_SAMPLES: &[&str] = &[
    "/r/a.js", "/r/a.jsx", "/r/a.ts", "/r/a.tsx", "/r/a.cjs", "/r/a.cts",
];

fn hoists(request: &str) -> bool {
    let r = request.strip_suffix(".js").unwrap_or(request);
    if r == NEXT_JEST_TRANSFORMER || r.ends_with(&format!("/node_modules/{NEXT_JEST_TRANSFORMER}"))
    {
        return true;
    }
    HOISTING_TRANSFORMERS.iter().any(|p| {
        request == *p
            || request
                .strip_prefix(p)
                .is_some_and(|rest| rest.starts_with('/'))
            || request.contains(&format!("/node_modules/{p}/"))
    })
}

/// Node builtins a transformer file uses for cache keys and paths.
const WRAPPER_BUILTINS: &[&str] = &[
    "assert", "buffer", "crypto", "fs", "module", "os", "path", "process", "url", "util",
];

/// Whether the first-party transformer `file` hands code to a transformer that
/// hoists mock calls: it loads one ([`hoists`]) and besides it only Babel
/// (`@babel/core`, plugins and presets), `package.json` files (versions for its
/// cache key) and a few Node builtins. A pass of Babel plugins before or after
/// the hoisting transformer does not move a hoisted call back below the imports
/// (the runtime mock oracle, runner `jw`). Any other load (another compiler:
/// `@swc/core`, esbuild, sucrase, TypeScript; a relative or computed
/// one) may be what produces the code instead: not known to hoist (runner
/// `jx`).
fn wrapper_hoists(file: &Path) -> bool {
    use turborepo_trace_core::RefKind;
    let Ok(src) = fsx::read(file) else {
        return false;
    };
    let ex = turborepo_trace_lang_js::scan(
        &src,
        turborepo_trace_lang_js::Dialect::from_path(file),
        turborepo_trace_lang_js::ScanOptions::default(),
    );
    let mut found = false;
    for r in &ex.refs {
        if !matches!(
            r.kind,
            RefKind::Import | RefKind::Require | RefKind::DynamicImport | RefKind::ReExport
        ) {
            continue;
        }
        let Some(spec) = r.spec.as_literal() else {
            return false;
        };
        let bare = spec.strip_prefix("node:").unwrap_or(spec);
        if hoists(spec) {
            found = true;
        } else if !(WRAPPER_BUILTINS.contains(&bare)
            || spec.ends_with("/package.json")
            || spec == "@babel/core"
            || [
                "babel-plugin-",
                "babel-preset-",
                "@babel/plugin-",
                "@babel/preset-",
            ]
            .iter()
            .any(|p| spec.starts_with(p)))
        {
            return false;
        }
    }
    found
}

/// Why the jest `transform` table `tv` of a project with root `root` may leave
/// mock calls where they are written: an entry for code files that is not a
/// transformer known to hoist them (a package of [`HOISTING_TRANSFORMERS`], or
/// a file of the repository that only wraps one, [`wrapper_hoists`]), or a
/// table that is not fully known. `None` for no table (jest's default is
/// `babel-jest`) and for a table of known transformers; entries whose pattern
/// matches no code file (style and asset stubs) do not count.
pub fn transform_keeps_mocks(tv: Option<&ConfigVal>, root: &Path) -> Option<String> {
    for alt in tv.map(ConfigVal::alternatives).unwrap_or_default() {
        match alt {
            ConfigVal::Null => {}
            ConfigVal::Obj(o) => {
                if o.open.is_some() {
                    return Some("the transform table is not fully known".into());
                }
                for (pattern, v) in &o.entries {
                    let re = crate::JsRegex::new(pattern.clone(), "");
                    if re.is_supported() && !CODE_SAMPLES.iter().any(|s| re.is_match(s)) {
                        continue;
                    }
                    for a in v.alternatives() {
                        let module = match a {
                            ConfigVal::Arr(xs) => xs.first(),
                            a => Some(a),
                        };
                        let request = match module {
                            Some(ConfigVal::Str(Value::Str(s))) => s.as_str(),
                            Some(ConfigVal::Resolve { spec, .. }) => spec.as_str(),
                            _ => return Some(format!("the transform of '{pattern}' is not known")),
                        };
                        let local = PathBuf::from(crate::common::replace_root_dir(request, root));
                        let wrapper = local.is_absolute()
                            && !fsx::in_node_modules(&local)
                            && wrapper_hoists(&local);
                        if !hoists(request) && !wrapper {
                            return Some(format!(
                                "the transform '{request}' is not known to hoist mock calls"
                            ));
                        }
                    }
                }
            }
            _ => return Some("the transform table is not known".into()),
        }
    }
    None
}

/// Adds to `out` what `file` (with the mock calls `scan`) replaces, loads for
/// real and leaves open. `runner` is the framework id (`jest`, `vitest`): calls
/// of the other runner's object are ignored, as they are by the runner (`""`
/// keeps all). `replaces`: the file is a test or a setup file, where a hoisted
/// top-level call replaces the module for the whole test; a helper's calls only
/// count against replacements.
pub fn mock_inputs(
    project: Option<&Project>,
    runner: &str,
    file: &Path,
    scan: &MockScan,
    replaces: bool,
    resolver: &dyn ModuleResolver,
    out: &mut TestInputs,
) {
    if scan.is_empty() {
        return;
    }
    let cx = Cx {
        project,
        resolver,
        dir: file.parent().unwrap_or(Path::new("/")),
        modules: Default::default(),
    };
    let hazard = |out: &mut TestInputs, what: String, line: u32| {
        let h = MockHazard { what, line };
        if !out.mock_hazards.contains(&h) {
            out.mock_hazards.push(h);
        }
    };
    // The module of `request` is loaded for real (or may be). `eager`: when the
    // file's code runs (`requireActual`); else when something requests the
    // module (the source of an automock, a factory that takes the original, a
    // module that was unmocked).
    let real = |out: &mut TestInputs, what: &str, request: &str, line: u32, eager: bool| {
        let add = |out: &mut TestInputs, p: PathBuf| {
            if eager && !out.loaded_eager.contains(&p) {
                out.loaded_eager.push(p.clone());
            }
            if !out.loaded_real.contains(&p) {
                out.loaded_real.push(p);
            }
        };
        match cx.module(request) {
            Module::File(p) => {
                add(out, p);
                None
            }
            Module::Many(ps) => {
                for p in ps {
                    add(out, p);
                }
                None
            }
            Module::Package => None,
            Module::Unknown => Some((format!("{what}('{request}') does not resolve"), line)),
        }
    };
    // A manual mock that jest finds by the request's name anywhere in the roots:
    // not the one next to the module.
    let haste = |r: &str| project.is_some_and(|p| p.mocks.haste_mocks.iter().any(|(k, _)| k == r));
    let mine = |c: &MockCall| runner.is_empty() || c.api.runner == runner;
    let mut replaced: Vec<Replaced> = Vec::new();
    for c in scan.calls.iter().filter(|c| mine(c)) {
        let name = c.name();
        // A helper's mock without a factory (a test's own are part of its framework
        // inputs). The call loads nothing: the runner loads the manual mock, or the
        // real module for an automock, when the module is first requested
        // afterwards. For a first-party module that is a manual mock the tests
        // load that request the module (`lazy_mocks`, below), or the module
        // itself, which a test that requests it depends on anyway.
        // `requireMock` loads at once, and a mock found by name or of a package
        // is not tied to one module file: a dependency of the helper.
        if !replaces
            && c.arg == MockArg::Bare
            && matches!(
                c.api.role,
                MockRole::Replace | MockRole::ReplaceLate | MockRole::MockOf
            )
            && let (Some(p), Some(r)) = (project, &c.request)
            && (c.api.role == MockRole::MockOf
                || haste(r)
                || !matches!(cx.module(r), Module::File(_)))
        {
            MockCx {
                project: p,
                resolver,
                test_dir: cx.dir,
            }
            .mocked(r, &name, out);
        }
        let open = match (&c.request, c.api.role) {
            (_, MockRole::Registry) => None,
            (_, MockRole::Automock) => Some((format!("{name}()"), c.line)),
            (Some(r), MockRole::Replace | MockRole::ReplaceLate) => match c.arg {
                MockArg::Factory => {
                    if replaces
                        && c.api.role == MockRole::Replace
                        && c.top_level
                        && let Module::File(module) = cx.module(r)
                        && !project.is_some_and(|p| p.resolution.condition_requests.contains(r))
                    {
                        replaced.push(Replaced {
                            module,
                            call: name.clone(),
                            request: r.clone(),
                            line: c.line,
                            by_path: cx.by_path(r),
                            manual: None,
                        });
                    }
                    None
                }
                MockArg::Bare => match cx.module(r) {
                    Module::File(module) => match manual_mock(&module) {
                        Some(manual) => {
                            if replaces && c.api.role == MockRole::Replace && c.top_level {
                                replaced.push(Replaced {
                                    module,
                                    call: name.clone(),
                                    request: r.clone(),
                                    line: c.line,
                                    by_path: cx.by_path(r),
                                    manual: Some(manual),
                                });
                            } else if !replaces && !haste(r) {
                                // In vitest the later call wins: a test's factory for the
                                // module does not keep the manual mock out (in jest the
                                // factory stays). The module is not cut for the tests
                                // that load this file, so its edge to the mock is reached.
                                if runner != "jest" && !out.loaded_real.contains(&module) {
                                    out.loaded_real.push(module.clone());
                                }
                                let l = LazyMock {
                                    mock: manual,
                                    module,
                                };
                                if !out.lazy_mocks.contains(&l) {
                                    out.lazy_mocks.push(l);
                                }
                            }
                            None
                        }
                        // An automock is built from the real module, when it is requested.
                        None => real(out, &name, r, c.line, false),
                    },
                    Module::Many(_) => real(out, &name, r, c.line, true),
                    Module::Package => None,
                    Module::Unknown => Some((format!("{name}('{r}') does not resolve"), c.line)),
                },
                // The factory runs when the module is requested.
                MockArg::FactoryWithParam | MockArg::Options | MockArg::Other => {
                    real(out, &name, r, c.line, false)
                }
            },
            (None, MockRole::Replace | MockRole::ReplaceLate) => (c.arg != MockArg::Factory)
                .then(|| (format!("{name}(...) of a computed module"), c.line)),
            // `deepUnmock` also unmocks what the module loads.
            (Some(r), MockRole::Unmock) if c.api.method != "deepUnmock" => {
                real(out, &name, r, c.line, false)
            }
            (Some(r), MockRole::Actual | MockRole::MockOf) => {
                // In the factory of a mock of the same module (a partial mock): the
                // factory runs when the module is requested.
                let in_own_factory = c.api.role == MockRole::Actual
                    && c.within.as_deref().is_some_and(|w| {
                        w == r
                            || matches!(
                                (cx.module(w), cx.module(r)),
                                (Module::File(a), Module::File(b)) if a == b
                            )
                    });
                real(out, &name, r, c.line, !in_own_factory)
            }
            (Some(_), MockRole::Unmock) => Some((format!("{name}(...)"), c.line)),
            (None, MockRole::Unmock | MockRole::Actual | MockRole::MockOf) => {
                Some((format!("{name}(...) of a computed module"), c.line))
            }
        };
        if let Some((what, line)) = open {
            hazard(out, what, line);
        }
    }
    for l in &scan.loose {
        let open = match &l.request {
            Some(r) if l.name != "deepUnmock" => {
                // `x.mock('./m')` on a receiver that may be the runner's object: the manual
                // mock next to the module, as for a call that is read.
                if !replaces
                    && matches!(l.name.as_str(), "mock" | "doMock")
                    && !haste(r)
                    && let Module::File(module) = cx.module(r)
                    && let Some(mock) = manual_mock(&module)
                {
                    let l = LazyMock { mock, module };
                    if !out.lazy_mocks.contains(&l) {
                        out.lazy_mocks.push(l);
                    }
                }
                real(out, &l.name, r, l.line, true)
            }
            _ => Some((format!("{} used indirectly", l.name), l.line)),
        };
        if let Some((what, line)) = open {
            hazard(out, what, line);
        }
    }
    if replaced.is_empty() {
        return;
    }
    if runner == "jest"
        && let Some(p) = project
        && jest_esm(file, &p.root_dir)
    {
        hazard(
            out,
            "an ES module: jest.mock does not replace what it imports".into(),
            0,
        );
    }
    for r in replaced {
        // The same file loads the module for real: not replaced.
        if !out.loaded_real.contains(&r.module) && !out.replaced.contains(&r) {
            out.replaced.push(r);
        }
    }
}

#[cfg(test)]
mod tests {
    use turborepo_trace_lang_js::{Dialect, mocks::scan};

    use super::*;
    use crate::SimpleResolver;

    fn inputs(dir: &Path, src: &str, replaces: bool) -> TestInputs {
        let mut out = TestInputs::default();
        let file = dir.join("a.test.ts");
        mock_inputs(
            None,
            "jest",
            &file,
            &scan(src.as_bytes(), Dialect::TS),
            replaces,
            &SimpleResolver {
                extensions: vec!["ts".into()],
            },
            &mut out,
        );
        out
    }

    fn project() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for f in ["m.ts", "n.ts", "o.ts", "__mocks__/n.ts"] {
            let p = d.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "export const v = 1;\n").unwrap();
        }
        d
    }

    fn names(dir: &Path, paths: impl IntoIterator<Item = PathBuf>) -> Vec<String> {
        let dir = fsx::normalize(&std::fs::canonicalize(dir).unwrap());
        paths
            .into_iter()
            .map(|p| {
                let p = std::fs::canonicalize(&p).unwrap_or(p);
                p.strip_prefix(&dir).unwrap_or(&p).display().to_string()
            })
            .collect()
    }

    #[test]
    fn a_top_level_factory_or_manual_mock_replaces() {
        let d = project();
        let t = inputs(
            d.path(),
            "jest.mock('./m', () => ({ v: 2 \
             }));\njest.mock('./n');\njest.mock('./o');\njest.mock('react', () => ({}));\n",
            true,
        );
        let got: Vec<(String, u32, bool)> = t
            .replaced
            .iter()
            .map(|r| (r.request.clone(), r.line, r.manual.is_some()))
            .collect();
        assert_eq!(got, [("./m".into(), 1, false), ("./n".into(), 2, true)]);
        // `./o` has no manual mock: the automock loads it.
        assert_eq!(names(d.path(), t.loaded_real.clone()), ["o.ts"]);
        assert!(t.mock_hazards.is_empty(), "{:?}", t.mock_hazards);
    }

    #[test]
    fn transforms_that_hoist() {
        let table = |entries: &[(&str, ConfigVal)]| {
            ConfigVal::Obj(turborepo_trace_lang_js::extract::config::ConfigObj {
                entries: entries
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect(),
                open: None,
            })
        };
        let s = |v: &str| ConfigVal::Str(Value::Str(v.into()));
        let code = r"^.+\.(t|j)sx?$";
        // No table: jest's default, babel-jest.
        assert_eq!(transform_keeps_mocks(None, Path::new("/repo")), None);
        for known in [
            "@swc/jest",
            "babel-jest",
            "ts-jest",
            "ts-jest/legacy",
            "next/dist/build/swc/jest-transformer",
            "/repo/node_modules/@swc/jest/index.js",
        ] {
            assert_eq!(
                transform_keeps_mocks(Some(&table(&[(code, s(known))])), Path::new("/repo")),
                None,
                "{known}"
            );
        }
        // With options; a stub for files that are not code next to it.
        let t = table(&[
            (code, ConfigVal::Arr(vec![s("@swc/jest"), table(&[])])),
            (r"\.(css|svg)$", s("/repo/stub.js")),
        ]);
        assert_eq!(transform_keeps_mocks(Some(&t), Path::new("/repo")), None);
        for unknown in ["esbuild-jest", "/repo/transform.js", "@swc/core"] {
            assert!(
                transform_keeps_mocks(Some(&table(&[(code, s(unknown))])), Path::new("/repo"))
                    .is_some(),
                "{unknown}"
            );
        }
        // A file of the repository that only wraps a hoisting transformer; one that
        // compiles by itself; one that loads another file.
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("wrap.cjs"),
            "const { transformSync } = require('@babel/core');\nconst { createTransformer } = \
             require('next/dist/build/swc/jest-transformer');\nconst v = \
             require('next/package.json').version;\nconst { createHash } = \
             require('node:crypto');\nmodule.exports = { createTransformer };\n",
        )
        .unwrap();
        std::fs::write(
            d.path().join("own.cjs"),
            "const { transformSync } = require('@swc/core');\nconst { createTransformer } = \
             require('@swc/jest');\nmodule.exports = { process: (s, f) => transformSync(s) };\n",
        )
        .unwrap();
        std::fs::write(
            d.path().join("rel.cjs"),
            "module.exports = require('./other');\nrequire('@swc/jest');\n",
        )
        .unwrap();
        for (file, ok) in [
            ("<rootDir>/wrap.cjs", true),
            ("<rootDir>/own.cjs", false),
            ("<rootDir>/rel.cjs", false),
            ("<rootDir>/missing.cjs", false),
        ] {
            assert_eq!(
                transform_keeps_mocks(Some(&table(&[(code, s(file))])), d.path()).is_none(),
                ok,
                "{file}"
            );
        }
        // An entry that is not a string, a table with unknown keys.
        assert!(
            transform_keeps_mocks(
                Some(&table(&[(code, ConfigVal::Unknown("f()".into()))])),
                Path::new("/repo")
            )
            .is_some()
        );
        let mut open = table(&[(code, s("@swc/jest"))]);
        if let ConfigVal::Obj(o) = &mut open {
            o.open = Some("...spread".into());
        }
        assert!(transform_keeps_mocks(Some(&open), Path::new("/repo")).is_some());
    }

    #[test]
    fn what_loads_the_real_module_undoes_it() {
        let d = project();
        for undo in [
            "const real = jest.requireActual('./m');",
            "jest.unmock('./m');",
            "jest.dontMock('./m');",
            "jest.doMock('./m');",
            "jest.mock('./m', factory);",
            "x.mock('./m');",
            "jest.requireMock('./m');",
        ] {
            let t = inputs(
                d.path(),
                &format!("jest.mock('./m', () => ({{}}));\n{undo}\n"),
                true,
            );
            assert!(t.replaced.is_empty(), "{undo}: {:?}", t.replaced);
            assert_eq!(names(d.path(), t.loaded_real.clone()), ["m.ts"], "{undo}");
        }
        // Not undone: the registry is cleared, the mock stays; a late factory.
        for keep in [
            "jest.resetModules();",
            "jest.isolateModules(() => require('./m'));",
            "jest.doMock('./m', () => ({}));",
            "beforeEach(() => { jest.mock('./m', () => ({})); });",
        ] {
            let t = inputs(
                d.path(),
                &format!("jest.mock('./m', () => ({{}}));\n{keep}\n"),
                true,
            );
            assert_eq!(t.replaced.len(), 1, "{keep}");
            assert!(
                t.loaded_real.is_empty() && t.mock_hazards.is_empty(),
                "{keep}"
            );
        }
    }

    #[test]
    fn calls_that_are_not_hoisted_statements_replace_nothing() {
        let d = project();
        for src in [
            "jest.doMock('./m', () => ({}));",
            "beforeAll(() => { jest.mock('./m', () => ({})); });",
            "if (x) { jest.mock('./m', () => ({})); }",
            "jest.mock('./m', (orig) => orig());",
            "jest.mock('./m', { spy: true });",
            "vi.mock('./m', () => ({}));",
        ] {
            let t = inputs(d.path(), src, true);
            assert!(t.replaced.is_empty(), "{src}: {:?}", t.replaced);
        }
        // A helper replaces nothing, whatever it calls.
        let t = inputs(d.path(), "jest.mock('./m', () => ({}));", false);
        assert!(t.replaced.is_empty() && t.loaded_real.is_empty() && t.mock_hazards.is_empty());
    }

    #[test]
    fn a_helpers_mock_without_a_factory_loads_when_the_module_is_requested() {
        let d = project();
        // `./n` has a manual mock, `./o` has none (an automock of the real module).
        let t = inputs(
            d.path(),
            "jest.mock('./n');\njest.doMock('./n');\njest.mock('./o');\nx.mock('./n');\n",
            false,
        );
        let lazy: Vec<(String, String)> = t
            .lazy_mocks
            .iter()
            .map(|l| {
                (
                    names(d.path(), [l.mock.clone()]).remove(0),
                    names(d.path(), [l.module.clone()]).remove(0),
                )
            })
            .collect();
        assert_eq!(lazy, [("__mocks__/n.ts".to_owned(), "n.ts".to_owned())]);
        assert!(t.replaced.is_empty() && t.edges.is_empty() && t.entries.is_empty());
        // The automock's source is loaded for real (no test's replacement of it holds),
        // but not by the helper's own code; the call on an unknown receiver may be
        // anything.
        assert_eq!(names(d.path(), t.loaded_real.clone()), ["o.ts", "n.ts"]);
        assert_eq!(names(d.path(), t.loaded_eager.clone()), ["n.ts"]);
        // What runs with the helper: `requireActual`, `requireMock`.
        let t = inputs(
            d.path(),
            "export const real = () => \
             jest.requireActual('./m');\njest.requireMock('./o');\njest.unmock('./n');\n",
            false,
        );
        assert_eq!(names(d.path(), t.loaded_eager.clone()), ["m.ts", "o.ts"]);
        assert_eq!(
            names(d.path(), t.loaded_real.clone()),
            ["m.ts", "o.ts", "n.ts"]
        );
        assert!(t.lazy_mocks.is_empty());
        // A partial mock: the real module loads when the factory runs, at the first
        // request for the module. In the factory of another module's mock: with the
        // file.
        let t = inputs(
            d.path(),
            "jest.mock('./m', () => ({ ...jest.requireActual('./m'), w: jest.requireActual('./o') \
             }));\n",
            false,
        );
        assert_eq!(names(d.path(), t.loaded_real.clone()), ["m.ts", "o.ts"]);
        assert_eq!(names(d.path(), t.loaded_eager.clone()), ["o.ts"]);
        // A test's own mock is not lazy: its edges are its framework inputs.
        let t = inputs(d.path(), "jest.mock('./n');\n", true);
        assert!(t.lazy_mocks.is_empty() && t.replaced.len() == 1);
    }

    #[test]
    fn what_cannot_be_read_is_a_hazard() {
        let d = project();
        for src in [
            "jest.requireActual(name);",
            "const { requireActual } = jest;",
            "jest.unmock(name);",
            "jest.deepUnmock('./m');",
            "jest.mock(name);",
            "jest.enableAutomock();",
            "jest.requireActual('./missing');",
        ] {
            let t = inputs(d.path(), src, false);
            assert_eq!(t.mock_hazards.len(), 1, "{src}: {:?}", t.mock_hazards);
        }
        // A computed module with a factory replaces more, never less.
        let t = inputs(d.path(), "jest.mock(name, () => ({}));", false);
        assert!(t.mock_hazards.is_empty());
    }
}
