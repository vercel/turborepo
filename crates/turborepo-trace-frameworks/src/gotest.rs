//! The `go test` adapter.
//!
//! A project per module (`go.mod`). `go test` compiles a test file together
//! with its whole package directory, so a test's inputs are:
//!
//! | Input | Shape |
//! |---|---|
//! | every file of the package directory: the package's `.go` files, the `_test.go` files of the package and of its external `_test` package, cgo/assembly/object files | entries (traced as Go entries, so their imports are followed), plus a `(pkg dir, "*")` membership: an added or deleted file changes the package |
//! | `testdata/**` (`go test` runs in the package directory, and `testdata` is the convention for test fixtures) | entries for the files (contents) plus a `(pkg dir/testdata, "**")` membership; a probe when `testdata` is missing |
//! | imports of every `_test.go` file of the directory | module-request entries resolved from the package directory (the Go resolver maps them to package files) |
//! | `go.mod`, `go.sum`, the governing `go.work`/`go.work.sum`, `vendor/modules.txt` | project config files |
//!
//! Files are selected for every target (`lang-go` with `all_targets`): CI may
//! run the same test on several platforms, so a change to `x_windows.go`
//! affects the test everywhere it could build. Discovery is `*_test.go` below
//! the module root, skipping `testdata`, `vendor`, directories starting with
//! `.` or `_`, and nested modules (as `./...` does).
//!
//! Roles: package files are [`InputRole::CompiledWithTest`], `testdata` files
//! [`InputRole::TestData`], imports of the test files
//! [`InputRole::ImportedByTest`]. The module is the project's and every test's
//! [`crate::PackageRef`] (`go`, module path).

use std::path::{Path, PathBuf};

use turborepo_trace_core::{Platform, UncertaintyKind};
use turborepo_trace_lang_go::{
    BuildContext, header::scan_go_header, modules::is_bundled_std, package::package_files,
};

use crate::{
    Framework, ModuleResolver, fsx,
    fsx::PathFsx,
    inputs::{
        FrameworkInputs, FrameworkUncertainty, GlobalInputs, InputRef, InputRole, MembershipSpec,
        PackageRef, Project, ResolutionOverrides, SnapshotPolicy, Target, TestDiscovery,
        TestInputs, TestPattern,
    },
};

/// The `go test` adapter.
#[derive(Copy, Clone, Debug, Default)]
pub struct GoTest;

const CONFIG_NAMES: &[&str] = &["go.mod"];

/// Maximum number of `testdata` files listed as entries; above it the
/// membership alone stands for them and an uncertainty is reported.
const MAX_TESTDATA: usize = 10_000;

/// Whether a directory is skipped by `./...` patterns.
fn skipped_dir(name: &str) -> bool {
    name == "testdata" || name == "vendor" || name.starts_with('.') || name.starts_with('_')
}

fn module_path(go_mod: &Path) -> Option<String> {
    let src = fsx::read_to_string(go_mod).ok()?;
    turborepo_trace_lockfile::gosum::parse_go_mod(&src)
        .ok()?
        .module
}

/// The `go.work` above `dir` (nearest), if any.
fn go_work_above(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .map(|d| d.join("go.work"))
        .find(|w| w.fx_is_file())
}

/// Directories of nested modules below `root` (not `root` itself), for
/// discovery excludes: the ones `./...` would reach (not below a skipped
/// directory), outermost only.
///
/// The `go.mod` files below the root say which they are, and the walk asks for
/// those alone ([`fsx::walk_files_matching`]): a caller that keeps the result
/// (the load of the module, for every Go file below it) depends on that
/// listing, not on the entries of every directory of the module.
fn nested_modules(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    fsx::walk_files_matching(root, "**/go.mod", &mut |p| {
        let Some(dir) = p.parent().filter(|d| *d != root) else {
            return;
        };
        let skipped = dir.strip_prefix(root).is_ok_and(|rel| {
            rel.components()
                .any(|c| skipped_dir(&c.as_os_str().to_string_lossy()))
        });
        if !skipped {
            dirs.push(dir.to_path_buf());
        }
    });
    dirs.sort();
    let all = dirs.clone();
    dirs.retain(|d| !all.iter().any(|o| o != d && d.starts_with(o)));
    dirs
}

/// Walks directories below `root` as `./...` does; `f` returns whether to
/// descend.
fn walk_go_dirs(root: &Path, f: &mut dyn FnMut(&Path) -> bool) {
    fn go(dir: &Path, depth: usize, f: &mut dyn FnMut(&Path) -> bool) {
        if depth > 64 || !f(dir) {
            return;
        }
        let Some(entries) = fsx::read_dir(dir) else {
            return;
        };
        for d in entries.into_iter().filter(|(_, d)| *d).map(|(p, _)| p) {
            let name = d.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !skipped_dir(name) && !fsx::skip_dir(name) {
                go(&d, depth + 1, f);
            }
        }
    }
    go(root, 0, f);
}

impl GoTest {
    /// The build context tests are selected for: the host, every target
    /// included.
    fn build_context() -> BuildContext {
        let mut cx = BuildContext::new(Platform::host());
        cx.all_targets = true;
        cx
    }
}

impl Framework for GoTest {
    fn id(&self) -> &'static str {
        "gotest"
    }

    /// `go test` runs the tests of a package in the package's directory.
    fn test_cwds(&self, _project: &Project, test: &Path) -> Vec<PathBuf> {
        test.parent().map(Path::to_path_buf).into_iter().collect()
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        CONFIG_NAMES
    }

    fn is_config_file(&self, path: &Path) -> bool {
        path.file_name().is_some_and(|n| n == "go.mod")
    }

    fn discover(&self, root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        walk_go_dirs(root, &mut |d| {
            let m = d.join("go.mod");
            if m.fx_is_file() {
                out.push(m);
            }
            true
        });
        out
    }

    fn config_for(&self, test: &Path) -> Option<PathBuf> {
        test.parent()?
            .ancestors()
            .map(|d| d.join("go.mod"))
            .find(|m| m.fx_is_file())
    }

    fn load(&self, config: &Path) -> FrameworkInputs {
        let root = config.parent().unwrap_or(Path::new("/")).to_path_buf();
        let mut uncertainties = Vec::new();
        let module = module_path(config);
        let package = module.as_ref().map(|m| PackageRef {
            ecosystem: "go",
            name: m.clone(),
            version: None,
            root: root.clone(),
        });
        let name = module.unwrap_or_else(|| {
            uncertainties.push(FrameworkUncertainty {
                kind: UncertaintyKind::ParseIncomplete,
                file: config.to_path_buf(),
                line: 0,
                field: Some("module".into()),
                detail: "go.mod has no readable module path".into(),
            });
            root.display().to_string()
        });
        let mut config_files = vec![config.to_path_buf()];
        let mut probes = Vec::new();
        let sum = root.join("go.sum");
        if sum.fx_is_file() {
            config_files.push(sum);
        } else {
            probes.push(sum);
        }
        if let Some(w) = go_work_above(&root) {
            let ws = w.with_file_name("go.work.sum");
            if ws.fx_is_file() {
                config_files.push(ws);
            }
            config_files.push(w);
        }
        let modules_txt = root.join("vendor/modules.txt");
        if modules_txt.fx_is_file() {
            config_files.push(modules_txt);
        }
        let mut exclude = vec![
            TestPattern::Glob("**/testdata/**".into()),
            TestPattern::Glob("vendor/**".into()),
            TestPattern::Glob("**/.*/**".into()),
            TestPattern::Glob("**/_*/**".into()),
        ];
        for n in nested_modules(&root) {
            if let Ok(rel) = n.strip_prefix(&root) {
                exclude.push(TestPattern::Glob(format!("{}/**", rel.display())));
            }
        }
        let project = Project {
            name,
            config_file: Some(config.to_path_buf()),
            root_dir: root.clone(),
            global: GlobalInputs {
                config_files,
                probes,
                package,
                ..GlobalInputs::default()
            },
            resolution: ResolutionOverrides::default(),
            discovery: TestDiscovery {
                roots: vec![root.clone()],
                include: vec![TestPattern::Glob("**/*_test.go".into())],
                exclude,
                glob_base: Some(root),
                extensions: vec!["go".into()],
                absolute: false,
                choice: None,
                spec: None,
            },
            snapshots: SnapshotPolicy::Default,
            uncertainties: Vec::new(),
            ..Project::default()
        };
        FrameworkInputs {
            framework: "gotest",
            config_file: Some(config.to_path_buf()),
            projects: vec![project],
            uncertainties,
        }
    }

    fn test_inputs(
        &self,
        project: &Project,
        test: &Path,
        src: &[u8],
        _resolver: &dyn ModuleResolver,
    ) -> TestInputs {
        let mut out = TestInputs {
            test: test.to_path_buf(),
            package: project.global.package.clone(),
            ..TestInputs::default()
        };
        let Some(dir) = test.parent() else {
            return out;
        };
        // (The scanner asks the cache itself.)
        let pf = fsx::direct(|fs| package_files(fs, dir, &Self::build_context()));
        // The whole package directory, every target.
        let mut files: Vec<&PathBuf> = pf
            .go
            .iter()
            .chain(&pf.tests)
            .chain(&pf.other)
            .chain(pf.conditional.iter().map(|c| &c.path))
            .filter(|p| p.as_path() != test)
            .collect();
        files.sort();
        files.dedup();
        out.entries.extend(files.into_iter().map(|p| InputRef {
            role: InputRole::CompiledWithTest,
            target: Target::Path(p.clone()),
        }));
        out.memberships.push(MembershipSpec {
            dir: dir.to_path_buf(),
            pattern: "*".into(),
            reason: "go test compiles every file of the package directory".into(),
        });
        // testdata/**.
        let td = dir.join("testdata");
        if td.fx_is_dir() {
            let mut n = 0usize;
            let mut truncated = false;
            fsx::walk_files(&td, &mut |p| {
                if n < MAX_TESTDATA {
                    out.entries.push(InputRef {
                        role: InputRole::TestData,
                        target: Target::Path(p.to_path_buf()),
                    });
                } else {
                    truncated = true;
                }
                n += 1;
            });
            if truncated {
                out.uncertainties.push(FrameworkUncertainty {
                    kind: UncertaintyKind::PatternTooBroad,
                    file: test.to_path_buf(),
                    line: 0,
                    field: None,
                    detail: format!(
                        "{} has {n} files; the first {MAX_TESTDATA} are entries, the membership \
                         covers the rest",
                        td.display()
                    ),
                });
            }
            out.memberships.push(MembershipSpec {
                dir: td,
                pattern: "**".into(),
                reason: "go test fixtures (testdata)".into(),
            });
        } else {
            out.probes.push(td);
        }
        // Imports of every test file of the directory (the given source for the test
        // itself, which may be unsaved or not yet on disk).
        let mut imports: Vec<String> = pf.test_imports.clone();
        imports.extend(scan_go_header(src).imports);
        for c in pf.conditional.iter().filter(|c| {
            c.path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with("_test.go"))
        }) {
            if let Some(h) = turborepo_trace_lang_go::package::read_go_header(&c.path) {
                imports.extend(h.imports);
            }
        }
        imports.sort();
        imports.dedup();
        for imp in imports {
            if imp == "C" || is_bundled_std(&imp) {
                continue;
            }
            out.entries.push(InputRef {
                role: InputRole::ImportedByTest,
                target: Target::Module {
                    request: imp,
                    from_dir: dir.to_path_buf(),
                },
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SimpleResolver;

    fn fixture() -> PathBuf {
        fsx::normalize(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../turborepo-trace-testkit/fixtures/go/basic"),
        )
    }

    #[test]
    fn project_and_discovery() {
        let root = fixture();
        let found = GoTest.discover(&root);
        assert!(found.contains(&root.join("go.mod")));
        assert!(found.contains(&root.join("lib/go.mod")));
        let inputs = GoTest.load(&root.join("go.mod"));
        let p = &inputs.projects[0];
        assert_eq!(p.name, "example.com/basic");
        assert!(p.global.config_files.contains(&root.join("go.sum")));
        let tests = p.discovery.enumerate();
        let rel: Vec<_> = tests
            .iter()
            .map(|t| {
                t.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        // Nested modules (lib/, the fake module cache) and testdata are not this
        // project's.
        assert_eq!(
            rel,
            [
                "internal/util/export_test.go",
                "internal/util/util_test.go",
                "main_test.go"
            ]
        );
        assert_eq!(
            GoTest.config_for(&root.join("internal/util/util_test.go")),
            Some(root.join("go.mod"))
        );
    }

    #[test]
    fn test_inputs_cover_package_testdata_and_imports() {
        let root = fixture();
        let inputs = GoTest.load(&root.join("go.mod"));
        let test = root.join("internal/util/util_test.go");
        let src = fsx::read(&test).unwrap();
        let ti = GoTest.test_inputs(&inputs.projects[0], &test, &src, &SimpleResolver::default());
        let paths: Vec<String> = ti
            .entries
            .iter()
            .filter_map(|e| match &e.target {
                Target::Path(p) => Some(p.strip_prefix(&root).unwrap().display().to_string()),
                Target::Module { .. } => None,
            })
            .collect();
        assert_eq!(
            paths,
            [
                "internal/util/asm_amd64.s",
                "internal/util/asm_arm64.s",
                "internal/util/export_test.go",
                "internal/util/ignored.go",
                "internal/util/tagged.go",
                "internal/util/util.go",
                "internal/util/util_darwin_arm64.go",
                "internal/util/util_linux.go",
                "internal/util/util_windows.go",
                "internal/util/testdata/golden.txt",
            ]
        );
        let modules: Vec<&str> = ti
            .entries
            .iter()
            .filter_map(|e| match &e.target {
                Target::Module { request, .. } => Some(request.as_str()),
                Target::Path(_) => None,
            })
            .collect();
        assert_eq!(modules, ["example.com/basic/internal/util"]);
        let m: Vec<_> = ti
            .memberships
            .iter()
            .map(|m| {
                (
                    m.dir.strip_prefix(&root).unwrap().display().to_string(),
                    m.pattern.as_str(),
                )
            })
            .collect();
        assert_eq!(
            m,
            [
                ("internal/util".to_string(), "*"),
                ("internal/util/testdata".to_string(), "**")
            ]
        );
        assert!(ti.probes.is_empty());
        // A package without testdata probes for it.
        let main_test = root.join("main_test.go");
        let ti = GoTest.test_inputs(
            &inputs.projects[0],
            &main_test,
            &fsx::read(&main_test).unwrap(),
            &SimpleResolver::default(),
        );
        assert_eq!(ti.probes, [root.join("testdata")]);
    }

    #[test]
    fn registered() {
        assert!(crate::framework("gotest").is_some());
    }
}
