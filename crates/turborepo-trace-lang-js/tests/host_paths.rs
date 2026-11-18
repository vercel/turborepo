//! Host paths: a graph is a function of the repository tree.
//!
//! In a confined trace (graph mode,
//! [`turborepo_trace_core::TraceOptions::confined`]) a path outside the base is
//! never looked up on the machine: a read, a listing or a glob of one is
//! an external reference, and a module loaded from one keeps a record
//! ([`turborepo_trace_core::resolve_path::assets`], "Host paths").
//!
//! The sources (`fixtures/host-paths/src`) have the shapes of real sites of two
//! large monorepos, with other names: a browser service listing `/tmp` to
//! delete stale profile directories, a test probing candidate shells, process
//! helpers reading `/proc/<pid>/stat`, a precompiler loading a native addon
//! from `/usr/local/lib`. Each test builds its own "machine": a temporary
//! directory with a repository (`repo/`) and host files next to it (`host/`),
//! which the source names instead of `/tmp`, `/proc` and `/bin`, so the old
//! behavior (the unconfined trace) can be shown on the same source. The whole
//! fixture, with the real paths, is built as a graph by the trace's own
//! command tests.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use turborepo_trace_core::{AnalyzeInput, FileAnalysis, Origin, RefKind, Tracer, UncertaintyKind};
use turborepo_trace_lang_js::JsLanguage;

/// `fixtures/host-paths/src/<file>` with the host directory `dir` it names
/// (`/tmp`, `/proc`, `/bin`) replaced by `HOST`, which [`Lab::analyze`] points
/// at the lab's host directory.
fn fixture(src: &str, dir: &str) -> String {
    src.replace(&format!("'{dir}"), "'HOST")
        .replace(&format!("`{dir}"), "`HOST")
}

/// A service that deletes stale browser profile directories lists `/tmp`. The
/// listing used to be expanded on the build machine: one edge and one node per
/// file that happened to be there (555 on one machine, 600 with forty more
/// files in `/tmp`).
fn listing() -> String {
    fixture(
        include_str!("../../turborepo-trace-testkit/fixtures/host-paths/src/profiles.js"),
        "/tmp",
    )
}

/// A helper that probes candidate shells (`/bin/sh`, `/bin/bash`, `/bin/dash`)
/// and skips the missing ones. The existing ones used to become nodes, through
/// the build machine's symlinks (`/bin/sh` was `/usr/bin/dash` there).
fn shells() -> String {
    fixture(
        include_str!("../../turborepo-trace-testkit/fixtures/host-paths/src/shells.js"),
        "/bin",
    )
}

/// Process helpers that list `/proc` and read `/proc/<pid>/stat`. The glob used
/// to be expanded on the build machine and its match count written into the
/// record (`/proc/*/stat: 38 match(es)`, 53 with a few more processes running).
fn proc() -> String {
    fixture(
        include_str!("../../turborepo-trace-testkit/fixtures/host-paths/src/proc.js"),
        "/proc",
    )
}

/// A precompiler that loads a native addon installed on the machine image.
const ADDON: &str = include_str!("../../turborepo-trace-testkit/fixtures/host-paths/src/addon.js");

/// The record of a listing of an absolute directory: in the reading in which
/// that directory is the place the repository is mounted at, it lists the
/// repository root.
const ROOT_LISTING: &str =
    "pattern_too_broad: directory . is the runtime working directory; not expanded";

/// A repository and a host directory next to it.
struct Lab {
    _dir: tempfile::TempDir,
    top: PathBuf,
}

impl Lab {
    /// In cargo's directory for test files, not in the temporary directory of
    /// the machine: the fixtures write the lab's path out, and a path below
    /// a temporary directory that the code writes out is a temporary file
    /// (its read keeps a record of its own).
    fn new() -> Lab {
        let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
        let top = dir.path().canonicalize().unwrap();
        let lab = Lab { _dir: dir, top };
        lab.write("repo/package.json", r#"{"name":"r","version":"1.0.0"}"#);
        lab.write("repo/src/data/a.json", "{}");
        lab.write("repo/src/data/b.json", "{}");
        lab.write("host/profile-1/lock", "");
        lab.write("host/a.txt", "");
        lab.write("host/sh", "");
        lab.write("host/41/stat", "");
        lab.write("host/42/stat", "");
        lab.write("host/addon.node", "");
        lab
    }

    fn write(&self, rel: &str, content: &str) {
        let p = self.top.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    fn repo(&self) -> PathBuf {
        self.top.join("repo")
    }

    /// The host directory, as source text.
    fn host(&self) -> String {
        self.top.join("host").display().to_string()
    }

    /// Whether this checkout, and so the lab, is itself below a temporary
    /// directory: the host paths of the fixtures are then temporary paths,
    /// and the tests about plain host paths have nothing to say.
    fn below_temp(&self) -> bool {
        let temp = turborepo_trace_core::fs::is_temp_path(&self.top);
        if temp {
            eprintln!(
                "skipped: {} is below a temporary directory",
                self.top.display()
            );
        }
        temp
    }

    /// Writes `repo/src/<name>` with `HOST` replaced by the host directory and
    /// analyzes it with a fresh tracer: `confined` as the graph store
    /// builds it, else as `trace` does.
    fn analyze(&self, name: &str, src: &str, confined: bool) -> Analysis {
        self.write(
            &format!("repo/src/{name}"),
            &src.replace("HOST", &self.host()),
        );
        let repo = self.repo();
        let tracer = Tracer::builder()
            .base(&repo)
            .cwd(&repo)
            .confined(confined)
            .record(true)
            .threads(1)
            .language(JsLanguage)
            .build()
            .unwrap();
        let a = tracer
            .analyze(vec![AnalyzeInput {
                path: repo.join("src").join(name),
                cached: None,
            }])
            .remove(0);
        Analysis {
            top: self.top.clone(),
            tracer,
            a,
        }
    }
}

struct Analysis {
    top: PathBuf,
    tracer: Tracer,
    a: FileAnalysis,
}

impl Analysis {
    fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.top)
            .map(|r| r.display().to_string())
            .unwrap_or_else(|_| p.display().to_string())
    }

    /// Every edge except the file-level ones: `kind spec -> targets [record
    /// kind]`, with targets relative to the lab directory.
    fn edges(&self) -> Vec<String> {
        self.a
            .edges
            .iter()
            .filter(|e| e.edge.kind != RefKind::Manifest)
            .map(|e| {
                let mut to: Vec<String> = e
                    .succ
                    .iter()
                    .map(|s| self.rel(&self.tracer.path_of(*s)))
                    .collect();
                to.sort();
                let spec = e
                    .edge
                    .spec
                    .replace(&self.top.display().to_string(), "<lab>");
                let unc = e
                    .edge
                    .uncertainty
                    .as_ref()
                    .map(|u| format!(" [{}]", u.kind.as_str()))
                    .unwrap_or_default();
                format!("{} {spec} -> {}{unc}", e.edge.kind, to.join(" "))
            })
            .collect()
    }

    /// Files reached below `host/`.
    fn host_targets(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .a
            .edges
            .iter()
            .flat_map(|e| e.succ.iter())
            .map(|s| self.rel(&self.tracer.path_of(*s)))
            .filter(|p| p.starts_with("host"))
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Paths below `host/` the resolution asked the file system about.
    fn host_observations(&self) -> Vec<String> {
        let host = self.top.join("host");
        let mut out = Vec::new();
        for g in &self.a.groups {
            let o = &g.observations;
            let paths = o.missing.iter().chain(&o.reads).chain(&o.path_reads);
            out.extend(paths.filter(|p| p.starts_with(&host)).map(|p| self.rel(p)));
            out.extend(
                o.listings
                    .iter()
                    .filter(|(d, _)| d.starts_with(&host))
                    .map(|(d, pat)| format!("{} {pat}", self.rel(d))),
            );
        }
        out.sort();
        out.dedup();
        out
    }

    fn details(&self) -> Vec<String> {
        self.a
            .edges
            .iter()
            .filter_map(|e| e.edge.uncertainty.as_ref())
            .map(|u| format!("{}: {}", u.kind.as_str(), u.detail))
            .collect()
    }

    /// Everything a shard would store for the file, as text.
    fn fingerprint(&self) -> String {
        format!("{:?}\n{:?}", self.a.edges, self.a.groups)
    }
}

#[test]
fn listing_a_host_directory_is_an_external_reference() {
    let lab = Lab::new();
    if lab.below_temp() {
        return;
    }
    let new = lab.analyze("profiles.js", &listing(), true);
    assert_eq!(
        new.edges(),
        [
            "require node:fs/promises -> ",
            "require node:path -> ",
            "fs_read_dir <lab>/host ->  [pattern_too_broad]",
        ]
    );
    assert_eq!(new.host_targets(), [] as [&str; 0]);
    assert_eq!(new.host_observations(), [] as [&str; 0]);
    // Nothing of the machine is looked at. The one record is the reading in which
    // the listed directory is the place the repository is mounted at: a listing
    // of the repository root, which is not expanded.
    assert_eq!(new.details(), [ROOT_LISTING]);

    // The unconfined trace (`trace`, and the graph before) expands it.
    let old = lab.analyze("profiles.js", &listing(), false);
    assert_eq!(
        old.host_targets(),
        [
            "host/41/stat",
            "host/42/stat",
            "host/a.txt",
            "host/addon.node",
            "host/profile-1/lock",
            "host/sh"
        ]
    );
    assert!(!old.host_observations().is_empty());
}

#[test]
fn the_analysis_does_not_depend_on_host_files() {
    let lab = Lab::new();
    let src = [&listing(), &shells(), &proc(), ADDON].join("\n");
    let before = lab.analyze("all.js", &src, true).fingerprint();
    let old_before = lab.analyze("all.js", &src, false).fingerprint();
    // Another machine: more files in the listed directory, another process table,
    // no shell, no addon.
    for i in 0..40 {
        lab.write(&format!("host/junk-{i}.txt"), "");
    }
    lab.write("host/43/stat", "");
    std::fs::remove_file(lab.top.join("host/sh")).unwrap();
    std::fs::remove_file(lab.top.join("host/addon.node")).unwrap();
    assert_eq!(lab.analyze("all.js", &src, true).fingerprint(), before);
    // Without confinement the same source gives another result on the other
    // machine.
    assert_ne!(lab.analyze("all.js", &src, false).fingerprint(), old_before);
}

#[test]
fn probing_host_files_reaches_no_node() {
    let lab = Lab::new();
    if lab.below_temp() {
        return;
    }
    let new = lab.analyze("shells.js", &shells(), true);
    assert_eq!(
        new.edges(),
        [
            "require node:fs -> ",
            "fs_read <lab>/host/sh | <lab>/host/bash | <lab>/host/dash -> ",
        ]
    );
    assert_eq!(new.host_observations(), [] as [&str; 0]);
    assert_eq!(new.details(), [] as [&str; 0]);
    let old = lab.analyze("shells.js", &shells(), false);
    assert_eq!(old.host_targets(), ["host/sh"]);
}

#[test]
fn a_host_glob_is_not_expanded_and_counts_nothing() {
    let lab = Lab::new();
    if lab.below_temp() {
        return;
    }
    let new = lab.analyze("proc.js", &proc(), true);
    assert_eq!(
        new.edges(),
        [
            "require node:fs -> ",
            "fs_read <lab>/host/*/stat ->  [generated_file]",
            "fs_read_dir <lab>/host ->  [pattern_too_broad]",
        ]
    );
    // The records hold what the code writes and nothing the machine has there. The
    // read has a computed directory directly below an absolute directory, which
    // may be the place the repository is mounted at:
    // it is not read as `*/stat` from the root, an assumption with a record of its
    // own (ignored by default, open under `--strict`). The listing has the
    // reading "the repository is mounted there".
    let details = new.details();
    assert_eq!(details.len(), 2, "{details:?}");
    assert!(
        details[0].starts_with("generated_file: ")
            && details[0].ends_with(&format!(
                "/host/*/stat {}",
                turborepo_trace_core::output::MOUNT_READ_NOTE
            )),
        "{details:?}"
    );
    assert_eq!(details[1], ROOT_LISTING);
    assert_eq!(new.host_observations(), [] as [&str; 0]);
    let old = lab.analyze("proc.js", &proc(), false);
    assert!(
        old.details().iter().any(|d| d.ends_with("2 match(es)")),
        "{:?}",
        old.details()
    );
    assert!(old.host_targets().contains(&"host/41/stat".to_owned()));
}

#[test]
fn a_module_loaded_from_outside_the_repository_keeps_a_record() {
    let lab = Lab::new();
    let new = lab.analyze("addon.js", ADDON, true);
    let records = new.details();
    assert!(
        records.iter().any(|d| d
            == "unresolved_specifier: /opt/tool/plugin.js is an absolute path: not looked up on \
                the build machine"),
        "{records:?}"
    );
    // The addon: one alternative is unknown (`options.addonPath`), the other a host
    // path.
    assert!(
        records
            .iter()
            .any(|d| d.contains("bytecode-precompiler-node") || d.contains("addonPath")),
        "{records:?}"
    );
    assert!(new.host_targets().is_empty());

    // A module in a temporary directory was generated at run time, from files the
    // site does not name: an open record with origin data, whether the path is
    // spelled out or comes from `os.tmpdir()`.
    let tmp = lab.analyze(
        "generated.js",
        "module.exports = require('/tmp/build-out/bundle.cjs');\n",
        true,
    );
    let u: Vec<_> = tmp
        .a
        .edges
        .iter()
        .filter_map(|e| e.edge.uncertainty.as_ref())
        .collect();
    assert_eq!(u.len(), 1, "{u:?}");
    assert_eq!(u[0].kind, UncertaintyKind::ComputedSpecifier);
    assert_eq!(u[0].origin, Origin::Data);
    assert!(
        u[0].detail.contains("temporary directory"),
        "{}",
        u[0].detail
    );
    assert!(u[0].scope.is_none());
}

#[test]
fn paths_inside_the_repository_and_links_out_of_it_still_resolve() {
    let lab = Lab::new();
    // A directory of the repository that is a link to a directory outside it: the
    // path is spelled inside the repository, so it is the repository's.
    turborepo_trace_core::os::symlink(lab.top.join("host"), lab.repo().join("src/linked")).unwrap();
    let src = r#"
const { readdirSync, readFileSync } = require('node:fs');
const path = require('node:path');
module.exports = [
  readdirSync(path.join(__dirname, 'data')),
  readFileSync(path.join(__dirname, 'linked', 'a.txt')),
];
"#;
    let a = lab.analyze("inside.js", src, true);
    assert_eq!(
        a.edges(),
        [
            "require node:fs -> ",
            "require node:path -> ",
            "fs_read_dir data -> repo/src/data/a.json repo/src/data/b.json",
            // The node is the real file, as before.
            "fs_read linked/a.txt -> host/a.txt",
        ]
    );
}

#[test]
fn the_directories_above_the_repository_are_the_machine_s() {
    let lab = Lab::new();
    if lab.below_temp() {
        return;
    }
    // The lab's directory contains the repository, as `/tmp` or `/home` contain a
    // checkout on some machines: listing it is the same external reference as
    // anywhere else, so the graph does not depend on where the checkout lives.
    let top = lab.top.display().to_string();
    let src = "const fs = require('node:fs');\nmodule.exports = [fs.readdirSync('TOP'), \
               fs.readdirSync('/'), (name) => fs.readFileSync('TOP/cache-' + name)];\n";
    let a = lab.analyze("above.js", &src.replace("TOP", &top), true);
    assert_eq!(
        a.edges(),
        [
            "require node:fs -> ",
            "fs_read_dir <lab> ->  [pattern_too_broad]",
            "fs_read_dir / -> ",
            "fs_read <lab>/cache-* ->  [partial_specifier]",
        ]
    );
    assert_eq!(a.details(), [ROOT_LISTING, "partial_specifier: name"]);

    // A pattern that climbs out of the repository by relative steps, or starts at
    // `/`, can lead back into it: it keeps its record, whose text names no
    // directory of the machine.
    let src = "const fs = require('node:fs');\nconst path = require('node:path');\nmodule.exports \
               = (name) => [\n  fs.readFileSync(path.join(__dirname, '../..', name)),\n  \
               fs.readFileSync('/' + name),\n];\n";
    let a = lab.analyze("climb.js", src, true);
    let d = a.details();
    assert_eq!(d.len(), 2, "{d:?}");
    for d in &d {
        assert!(d.starts_with("pattern_too_broad: "), "{d}");
        assert!(!d.contains(&top), "{d}");
    }
    assert!(
        d[0].ends_with("climbs above the repository root (to ..); not expanded"),
        "{d:?}"
    );
    assert!(
        d[1].ends_with("starts at the filesystem root; not expanded"),
        "{d:?}"
    );
    // Nothing next to the checkout was looked at.
    assert_eq!(a.host_targets(), [] as [&str; 0]);
    assert_eq!(a.host_observations(), [] as [&str; 0]);
}

/// The manifests that the resolutions of `rel` (a file of the lab's repository,
/// analyzed as the graph store does: confined) read through a view (the
/// resolution view, or the view of a config loader), relative to the lab
/// directory.
fn manifest_views(lab: &Lab, rel: &str) -> Vec<String> {
    let repo = lab.repo();
    let tracer = Tracer::builder()
        .base(&repo)
        .cwd(&repo)
        .confined(true)
        .record(true)
        .threads(1)
        .language(JsLanguage)
        .build()
        .unwrap();
    let a = tracer
        .analyze(vec![AnalyzeInput {
            path: repo.join(rel),
            cached: None,
        }])
        .remove(0);
    let mut out: Vec<String> = Vec::new();
    for g in &a.groups {
        let o = &g.observations;
        // (Through the resolution view, or through the view of a config loader.)
        let all = o.view_reads.iter().chain(&o.loader_reads).chain(
            o.key
                .iter()
                .flat_map(|k| k.view_reads.iter().chain(&k.loader_reads)),
        );
        for p in all {
            out.push(
                p.strip_prefix(&lab.top)
                    .map(|r| r.display().to_string())
                    .unwrap_or_else(|_| p.display().to_string()),
            );
        }
    }
    out.sort();
    out.dedup();
    out
}

const CFG_MANIFEST: &str = r#"{"name":"@cfg/base","version":"1.0.0"}"#;

/// A tsconfig that extends a config of a package is loaded through the manifest
/// of the package directory that holds it. The manifests on the way up to that
/// level are part of the record (an `exports` map there decides what loads).
/// The levels above it are not asked, so they are not recorded, and neither is
/// anything above the repository: where a checkout lives, and what is installed
/// next to it, is the machine's.
#[test]
fn a_config_extended_through_a_package_records_the_levels_that_can_answer() {
    let project = |lab: &Lab| {
        lab.write(
            "repo/packages/a/package.json",
            r#"{"name":"a","version":"1.0.0"}"#,
        );
        lab.write(
            "repo/packages/a/tsconfig.json",
            r#"{"extends":"@cfg/base/tsconfig.json"}"#,
        );
        lab.write("repo/packages/a/src/x.ts", "import './y';\n");
        lab.write("repo/packages/a/src/y.ts", "export {};\n");
    };
    let install = |lab: &Lab, dir: &str| {
        lab.write(
            &format!("{dir}node_modules/@cfg/base/package.json"),
            CFG_MANIFEST,
        );
        lab.write(&format!("{dir}node_modules/@cfg/base/tsconfig.json"), "{}");
    };
    let own = "repo/packages/a/node_modules/@cfg/base/package.json";
    let root = "repo/node_modules/@cfg/base/package.json";
    let above = "node_modules/@cfg/base/package.json";

    // Installed in the package, at the root and above the repository: the package's
    // own install answers.
    let lab = Lab::new();
    project(&lab);
    install(&lab, "repo/packages/a/");
    install(&lab, "repo/");
    install(&lab, "");
    let v = manifest_views(&lab, "packages/a/src/x.ts");
    assert!(v.iter().any(|p| p == own), "{v:?}");
    assert!(!v.iter().any(|p| p == root || p == above), "{v:?}");

    // At the root and above the repository: the root's answers.
    let lab = Lab::new();
    project(&lab);
    install(&lab, "repo/");
    install(&lab, "");
    let v = manifest_views(&lab, "packages/a/src/x.ts");
    assert!(v.iter().any(|p| p == root), "{v:?}");
    assert!(!v.iter().any(|p| p == above), "{v:?}");

    // Only above the repository: no manifest there is part of the record.
    let lab = Lab::new();
    project(&lab);
    install(&lab, "");
    let v = manifest_views(&lab, "packages/a/src/x.ts");
    assert!(!v.iter().any(|p| p == above), "{v:?}");
}

/// The `node_modules` walk of a package request records the manifest of a
/// package directory that was there and did not answer (the walk went on). Not
/// above the repository.
#[test]
fn a_package_directory_above_the_repository_is_not_part_of_a_walk_s_record() {
    // No entry and no index file: the directory does not answer.
    let silent = r#"{"name":"dep","version":"1.0.0"}"#;
    let lab = Lab::new();
    lab.write("repo/src/node_modules/dep/package.json", silent);
    lab.write("node_modules/dep/package.json", silent);
    lab.write("repo/src/walk.js", "require('dep');\n");
    let v = manifest_views(&lab, "src/walk.js");
    // The level inside the repository that the walk went through is recorded.
    assert!(
        v.iter()
            .any(|p| p == "repo/src/node_modules/dep/package.json"),
        "{v:?}"
    );
    assert!(
        !v.iter().any(|p| p == "node_modules/dep/package.json"),
        "{v:?}"
    );
}
