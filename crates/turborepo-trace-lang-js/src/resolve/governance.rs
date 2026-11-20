//! Which files a `tsconfig.json` governed at another revision of the tree.
//!
//! A graph is built from the working tree: a file's record names the configs
//! its discovery read ([`super::tsconfig`]). A project that a config above the
//! file lists under `references`, that loads and does not include the file, is
//! no input of it: the file only keeps it as a key observation
//! (`turborepo_trace_core::record::key`). When such a project included the file
//! at the base of an `affected` query and no longer does, the file's imports
//! resolved through the project's `paths` before and resolve through
//! another config now, and nothing in the graph of the working tree says so.
//!
//! [`Compare`] answers that from the two trees: it runs the same discovery on
//! the working tree and on the base revision (the working tree with the diff's
//! files as they were, [`Overlay`]) and tells for a file whether the two
//! differ: another governing config, or other load failures at the levels the
//! file consults (a config that does not load is reported on every file that
//! consults it).
//!
//! [`governing_fields`] is the cheap test before it: a config whose `extends`,
//! `files`, `include`, `exclude` and `compilerOptions.allowJs` are the same at
//! both revisions loads or fails the same way and covers the same files,
//! whatever else changed in it.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use dashmap::DashMap;
use oxc_resolver::TsConfig;
use turborepo_trace_core::DirCache;

pub use super::overlay::{Overlay, ReadAt};
use super::{
    CachedFs, JsResolveOptions,
    overlay::Entry,
    tsconfig::{DirConfig, Governing, LevelDiff, Tsconfigs, Walk},
};

/// What of a tsconfig decides whether it loads and which files it covers.
#[derive(Debug, PartialEq, Eq)]
pub struct GoverningFields {
    extends: Vec<String>,
    files: Option<Vec<PathBuf>>,
    include: Option<Vec<PathBuf>>,
    exclude: Option<Vec<PathBuf>>,
    allow_js: Option<bool>,
}

/// The governing fields of the tsconfig `text`; `None` when the resolver's
/// parser rejects it (the config does not load).
pub fn governing_fields(text: &str) -> Option<GoverningFields> {
    let at = Path::new("/tsconfig.json");
    let c = TsConfig::parse(false, at, at, text.to_owned()).ok()?;
    Some(GoverningFields {
        extends: match c.extends {
            Some(oxc_resolver::ExtendsField::Single(s)) => vec![s],
            Some(oxc_resolver::ExtendsField::Multiple(v)) => v,
            None => Vec::new(),
        },
        files: c.files,
        include: c.include,
        exclude: c.exclude,
        allow_js: c.compiler_options.allow_js,
    })
}

/// How the discovery of a file differs between the base revision and the
/// working tree.
#[derive(Debug, PartialEq, Eq)]
pub struct Moved {
    /// The governing config at the base (a config file, or a project reference
    /// as its config lists it); `None` when no config governed the file.
    pub base: Option<PathBuf>,
    /// The governing config in the working tree.
    pub head: Option<PathBuf>,
    /// The configs the file consults whose load failures differ.
    pub loads: Vec<PathBuf>,
}

/// Whether the tsconfig `text` at `path` extends a config of a package that is
/// installed nowhere above it. The tracer loads such a config without that base
/// ([`super::fs::redirect_missing_bases`]: a base in a third-party package
/// cannot define first-party path mappings); what else the base holds is not
/// known.
pub fn extends_uninstalled(path: &Path, text: &str) -> bool {
    super::fs::redirect_missing_bases(&DirCache::new(), path, text).is_some()
}

/// What the loader makes of a project config ([`Projects::load`]).
#[derive(Debug)]
pub struct Project {
    /// Why it does not load, when it does not (the rest is then empty).
    pub error: Option<String>,
    /// The config file that was loaded (as the loader names it, links
    /// resolved).
    pub config: PathBuf,
    /// The directory of the config that was loaded.
    pub dir: PathBuf,
    /// Its `files` and `include` (its own or the ones it inherits), as the
    /// loader made them: absolute, `${configDir}` replaced. `None`: not
    /// set.
    pub files: Option<Vec<PathBuf>>,
    /// See [`Project::files`].
    pub include: Option<Vec<PathBuf>>,
    /// Every file the load read, where its links lead: the config, the configs
    /// of its `extends` chain, the manifests a package request went
    /// through.
    pub read: Vec<PathBuf>,
}

/// Project configs as the tracer's loader loads them, in the working tree and
/// at a base revision: for rules that ask what a project is at a revision
/// (which file its `extends` names is the loader's answer, and nobody else's).
pub struct Projects {
    fs: Arc<DirCache>,
    head: Tsconfigs,
    base: Option<Tsconfigs>,
}

impl std::fmt::Debug for Projects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Projects").finish_non_exhaustive()
    }
}

impl Projects {
    /// `base`: the files of the base revision that differ from the working
    /// tree, when there is one to read.
    pub fn new(base: Option<Overlay>) -> Projects {
        // (Recording: what a load read is asked for.)
        let fs = Arc::new(DirCache::with_recording());
        Projects {
            head: Side::new(&fs, None).tsconfigs,
            base: base.map(|o| Side::new(&fs, Some(Arc::new(o))).tsconfigs),
            fs,
        }
    }

    /// The project config that `path` names (absolute; as a `references` entry
    /// is written: a file, or a directory with a `tsconfig.json`), at the
    /// base revision or in the working tree. `None`: at the base, without
    /// one.
    pub fn load(&self, at_base: bool, path: &Path) -> Option<Project> {
        let side = if at_base {
            self.base.as_ref()?
        } else {
            &self.head
        };
        let l = side.load(path);
        let real = |p: &PathBuf| self.fs.canonicalize(p).path;
        let mut read: Vec<PathBuf> = (l.observed.reads.iter())
            .chain(&l.observed.view_reads)
            .chain(&l.observed.loader_reads)
            .map(real)
            .collect();
        read.sort();
        read.dedup();
        Some(match &l.config {
            Ok(c) => Project {
                error: None,
                config: real(&c.path().to_path_buf()),
                dir: c.directory().to_path_buf(),
                files: c.files.clone(),
                include: c.include.clone(),
                read,
            },
            Err(e) => Project {
                error: Some(e.to_string()),
                config: PathBuf::new(),
                dir: PathBuf::new(),
                files: None,
                include: None,
                read,
            },
        })
    }
}

/// One revision's discovery.
struct Side {
    tsconfigs: Tsconfigs,
}

impl Side {
    fn new(fs: &Arc<DirCache>, overlay: Option<Arc<Overlay>>) -> Side {
        let cached = match overlay {
            Some(o) => CachedFs::with_overlay(Arc::clone(fs), o),
            None => CachedFs::new_shared(Arc::clone(fs)),
        };
        let fallback = JsResolveOptions::default().tsconfig_fallback;
        Side {
            tsconfigs: Tsconfigs::new(cached, fallback),
        }
    }
}

/// The config of one directory at both revisions.
struct Level {
    base: Option<Arc<DirConfig>>,
    head: Option<Arc<DirConfig>>,
    diff: LevelDiff,
}

/// What discovery chose for a file, and the load failures it reports on it.
#[derive(PartialEq, Eq)]
struct Outcome {
    governing: Option<PathBuf>,
    errors: Vec<(PathBuf, Vec<String>)>,
}

impl Outcome {
    fn of(w: &Walk) -> Outcome {
        let level = |l: usize| w.levels.get(l).and_then(|(_, dc)| dc.as_ref());
        let governing = match w.governing {
            Governing::None => None,
            // The same config resolves the file's imports either way.
            Governing::Level(l) | Governing::Fallback(l) => level(l).map(|dc| dc.path.clone()),
            Governing::Reference(l, r) => level(l).and_then(|dc| dc.ref_paths.get(r).cloned()),
        };
        // As `JsResolver::config_errors`: every level the walk consulted.
        let errors = w
            .levels
            .iter()
            .filter_map(|(_, dc)| dc.as_ref())
            .filter(|dc| !dc.errors.is_empty())
            .map(|dc| (dc.path.clone(), dc.errors.clone()))
            .collect();
        Outcome { governing, errors }
    }
}

/// Config discovery at the base revision and in the working tree, compared file
/// by file.
pub struct Compare {
    fs: Arc<DirCache>,
    head: Side,
    base: Side,
    /// The base revision's files where they differ from the working tree.
    overlay: Arc<Overlay>,
    levels: DashMap<PathBuf, Arc<Level>>,
    /// Per directory: its config and every config above it are the same at both
    /// revisions.
    clean: DashMap<PathBuf, bool>,
    /// Per directory: the directory of the nearest `package.json` (it bounds
    /// the fallback). The working tree's: a file below a manifest that came
    /// or went has the manifest's probe or read in its own record.
    packages: DashMap<PathBuf, Option<PathBuf>>,
}

impl std::fmt::Debug for Compare {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compare").finish_non_exhaustive()
    }
}

impl Compare {
    /// `base`: the files of the base revision that differ from the working
    /// tree.
    pub fn new(base: Overlay) -> Compare {
        let fs = Arc::new(DirCache::new());
        let overlay = Arc::new(base);
        Compare {
            head: Side::new(&fs, None),
            base: Side::new(&fs, Some(Arc::clone(&overlay))),
            overlay,
            fs,
            levels: DashMap::new(),
            clean: DashMap::new(),
            packages: DashMap::new(),
        }
    }

    /// Whether the directory `d` has a config file at one of the two revisions.
    /// Asked of the disk with one `stat` per name, not of the directory
    /// cache: most directories above the files of a repository hold no
    /// config, and a listing of each would cost more than everything else
    /// here.
    fn has_config(&self, d: &Path) -> bool {
        super::tsconfig::CONFIG_NAMES.iter().any(|name| {
            let p = d.join(name);
            p.is_file() || self.overlay.entry(&self.fs, &p) == Some(Entry::File)
        })
    }

    fn level(&self, d: &Path) -> Arc<Level> {
        if let Some(l) = self.levels.get(d) {
            return Arc::clone(&l);
        }
        let (head, base) = if self.has_config(d) {
            (
                self.head.tsconfigs.dir_config(&self.fs, d),
                self.base.tsconfigs.dir_config(&self.fs, d),
            )
        } else {
            (None, None)
        };
        let diff = match (&base, &head) {
            (None, None) => LevelDiff::Same,
            (Some(b), Some(h)) => b.diff(h),
            _ => LevelDiff::Whole,
        };
        let l = Arc::new(Level { base, head, diff });
        self.levels.insert(d.to_path_buf(), Arc::clone(&l));
        l
    }

    fn clean(&self, d: &Path) -> bool {
        if let Some(c) = self.clean.get(d) {
            return *c;
        }
        let c = self.level(d).diff == LevelDiff::Same && d.parent().is_none_or(|p| self.clean(p));
        self.clean.insert(d.to_path_buf(), c);
        c
    }

    fn package_dir(&self, d: &Path) -> Option<PathBuf> {
        if let Some(p) = self.packages.get(d) {
            return p.clone();
        }
        let found = if self.fs.is_file(&d.join("package.json")) {
            Some(d.to_path_buf())
        } else {
            d.parent().and_then(|p| self.package_dir(p))
        };
        self.packages.insert(d.to_path_buf(), found.clone());
        found
    }

    /// How the discovery of `file` (absolute, in the working tree) differs
    /// between the two revisions; `None` when it is the same.
    pub fn moved(&self, file: &Path) -> Option<Moved> {
        let dir = file.parent()?;
        if self.clean(dir) {
            return None;
        }
        // A level whose references differ changes nothing for a file that none of them
        // covers at either revision.
        let file_str = file.to_string_lossy();
        let mut cur = Some(dir);
        let mut look = false;
        while let Some(d) = cur {
            let l = self.level(d);
            let covers = |dc: &Option<Arc<DirConfig>>, r: &Path| {
                dc.as_ref()
                    .is_some_and(|dc| dc.reference_covers(r, file, &file_str))
            };
            look = match &l.diff {
                LevelDiff::Same => false,
                LevelDiff::Whole => true,
                LevelDiff::Refs(rs) => rs.iter().any(|r| covers(&l.base, r) || covers(&l.head, r)),
            };
            if look {
                break;
            }
            cur = d.parent();
        }
        if !look {
            return None;
        }
        let package = self.package_dir(dir);
        let walk = |side: &Side| side.tsconfigs.walk(&self.fs, file, package.as_deref());
        let (base, head) = (
            Outcome::of(&walk(&self.base)),
            Outcome::of(&walk(&self.head)),
        );
        if base == head {
            return None;
        }
        let mut loads: Vec<PathBuf> = Vec::new();
        fn at<'o>(o: &'o Outcome, config: &Path) -> Option<&'o Vec<String>> {
            o.errors.iter().find(|(c, _)| c == config).map(|(_, e)| e)
        }
        for (config, _) in base.errors.iter().chain(&head.errors) {
            if at(&base, config) != at(&head, config) && !loads.contains(config) {
                loads.push(config.clone());
            }
        }
        Some(Moved {
            base: base.governing,
            head: head.governing,
            loads,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, content: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    /// The base revision: `files` as `(path, contents at the base)`, `None` for
    /// a file the diff added.
    fn compare(root: &Path, files: &[(&str, Option<&str>)]) -> Compare {
        let contents: std::collections::HashMap<PathBuf, String> = files
            .iter()
            .filter_map(|(p, c)| Some((root.join(p), (*c)?.to_owned())))
            .collect();
        Compare::new(Overlay::new(
            files.iter().map(|(p, c)| (root.join(p), c.is_some())),
            Box::new(move |p| contents.get(p).map(|s| s.clone().into_bytes())),
        ))
    }

    const APP: &str = r#"{ "compilerOptions": { "paths": { "@lib/*": ["./v1/*"] } }, "references": [{ "path": "./tsconfig.lib.json" }] }"#;

    /// The temporary tree's directory with the links above it resolved, as a
    /// repository root is given (on macOS the temporary directory lies
    /// behind a link): the overlay names a revision's entries by their real
    /// paths.
    fn root(d: &tempfile::TempDir) -> PathBuf {
        turborepo_trace_core::fs::normalize(&std::fs::canonicalize(d.path()).unwrap())
    }

    fn tree() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(r, "package.json", "{}");
        write(r, "app/tsconfig.json", APP);
        write(r, "app/tsconfig.lib.json", r#"{ "include": ["lib"] }"#);
        write(r, "app/src/helper.ts", "");
        write(r, "app/other/o.ts", "");
        d
    }

    #[test]
    fn governing_fields_of_a_config() {
        let f = |t: &str| governing_fields(t);
        assert_eq!(
            f(r#"{ "include": ["src"], "compilerOptions": { "strict": true } }"#),
            f(r#"{ /* c */ "compilerOptions": { "paths": {} }, "include": ["src"], }"#)
        );
        assert_ne!(
            f(r#"{ "include": ["src"] }"#),
            f(r#"{ "include": ["lib"] }"#)
        );
        assert_ne!(f(r#"{ "include": ["src"] }"#), f(r#"{ "files": ["src"] }"#));
        assert_ne!(f("{}"), f(r#"{ "exclude": [] }"#));
        assert_ne!(f("{}"), f(r#"{ "extends": "./a.json" }"#));
        assert_ne!(f("{}"), f(r#"{ "compilerOptions": { "allowJs": true } }"#));
        assert!(f("{}").is_some());
        assert!(f("{ \"include\": [").is_none());
        assert!(f(r#"{ "compilerOptions": 3 }"#).is_none());
    }

    #[test]
    fn a_project_that_included_a_file_at_the_base() {
        let d = tree();
        let r = &root(&d);
        let c = compare(
            r,
            &[("app/tsconfig.lib.json", Some(r#"{ "include": ["src"] }"#))],
        );
        assert_eq!(
            c.moved(&r.join("app/src/helper.ts")),
            Some(Moved {
                base: Some(r.join("app/tsconfig.lib.json")),
                head: Some(r.join("app/tsconfig.json")),
                loads: Vec::new(),
            })
        );
        assert_eq!(c.moved(&r.join("app/other/o.ts")), None);
    }

    #[test]
    fn a_project_that_did_not_load_at_the_base() {
        let d = tree();
        let r = &root(&d);
        // It did not parse; it was not there; the config it extends was not there.
        for base in [
            vec![("app/tsconfig.lib.json", Some("{ \"include\": ["))],
            vec![("app/tsconfig.lib.json", None)],
        ] {
            let c = compare(r, &base);
            let m = c.moved(&r.join("app/other/o.ts")).expect("differs");
            assert_eq!(m.base, m.head);
            assert_eq!(m.loads, [r.join("app/tsconfig.json")]);
        }
        write(
            r,
            "app/tsconfig.lib.json",
            r#"{ "extends": "./base.json", "include": ["lib"] }"#,
        );
        write(r, "app/base.json", "{}");
        let c = compare(r, &[("app/base.json", None)]);
        assert!(c.moved(&r.join("app/other/o.ts")).is_some());
        // The extended config had other options: nothing differs.
        let c = compare(
            r,
            &[(
                "app/base.json",
                Some(r#"{ "compilerOptions": { "strict": true } }"#),
            )],
        );
        assert_eq!(c.moved(&r.join("app/other/o.ts")), None);
        assert_eq!(c.moved(&r.join("app/src/helper.ts")), None);
        // It had the `include` the project inherits.
        write(
            r,
            "app/tsconfig.lib.json",
            r#"{ "extends": "./base.json" }"#,
        );
        write(r, "app/base.json", r#"{ "include": ["lib"] }"#);
        let c = compare(r, &[("app/base.json", Some(r#"{ "include": ["src"] }"#))]);
        assert!(c.moved(&r.join("app/src/helper.ts")).is_some());
        assert_eq!(c.moved(&r.join("app/other/o.ts")), None);
    }

    #[test]
    fn a_deleted_project_is_there_at_the_base() {
        let d = tree();
        let r = &root(&d);
        std::fs::remove_file(r.join("app/tsconfig.lib.json")).unwrap();
        let c = compare(
            r,
            &[("app/tsconfig.lib.json", Some(r#"{ "include": ["src"] }"#))],
        );
        let m = c.moved(&r.join("app/src/helper.ts")).expect("differs");
        assert_eq!(m.base, Some(r.join("app/tsconfig.lib.json")));
        assert_eq!(m.head, Some(r.join("app/tsconfig.json")));
        assert_eq!(m.loads, [r.join("app/tsconfig.json")]);
    }
}
