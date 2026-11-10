//! Build fingerprints, effective fingerprints and their diff over
//! `fixtures/next-build/lab` (a trimmed real Turbopack build) and edited copies
//! of it.
//!
//! The copies model what two real builds differ in: rebuild noise (build id,
//! action ids, content-hash names), a changed module, stylesheet changes,
//! routing changes. The checks against real builds of mutated apps are the
//! lab's (turbotrace's `scripts/next-build-lab`).

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use turborepo_trace_frameworks::nextjs::build::{
    diff::{CssMode, diff},
    fingerprint::{Fingerprint, FingerprintOptions, fingerprint},
    invokes::{
        Found, Invocation, InvokeKind, InvokeRelation, InvokeTarget, apply, manifest_relation,
    },
    manifest::NextBuild,
};

const BUILD_ID: &str = "Al-SShdjXUxDRuupQCSZm";

fn lab() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../turborepo-trace-testkit/fixtures/next-build/lab")
}

fn load() -> NextBuild {
    let p = lab();
    NextBuild::load_dist(&p, &p.join("dot-next")).expect("fixture build")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), dst).unwrap();
        }
    }
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_dir() {
            files_under(&e.path(), out);
        } else {
            out.push(e.path());
        }
    }
}

/// A copy of the fixture build in a temporary project.
struct Copy {
    _tmp: tempfile::TempDir,
    project: PathBuf,
    dist: PathBuf,
}

impl Copy {
    fn new() -> Copy {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().canonicalize().unwrap().join("app");
        let dist = project.join("dot-next");
        copy_dir(&lab().join("dot-next"), &dist);
        Copy {
            _tmp: tmp,
            project,
            dist,
        }
    }

    /// Replaces `from` by `to` in every file under `sub` that contains
    /// `marker`; returns how many files changed.
    fn edit(&self, sub: &str, marker: &str, from: &str, to: &str) -> usize {
        let mut files = Vec::new();
        files_under(&self.dist.join(sub), &mut files);
        let mut n = 0;
        for f in files {
            let Ok(s) = std::fs::read_to_string(&f) else {
                continue;
            };
            if s.contains(marker) && s.contains(from) {
                std::fs::write(&f, s.replace(from, to)).unwrap();
                n += 1;
            }
        }
        n
    }

    /// Changes a server chunk that only `pathname` loads (library chunks are
    /// one-module stubs in the fixture).
    fn change_own_chunk(&self, pathname: &str) {
        let b = load();
        let of = |p: &str| -> Vec<String> {
            let r = b.routes.iter().find(|r| r.pathname == p).unwrap();
            r.trace.inside.clone()
        };
        let chunk = of(pathname)
            .into_iter()
            .find(|f| {
                f.contains("/chunks/")
                    && !f.contains("_actions_")
                    && b.routes
                        .iter()
                        .filter(|r| r.pathname != pathname)
                        .all(|r| !r.trace.inside.contains(f))
            })
            .expect("a chunk of its own");
        let path = self.dist.join(&chunk);
        let s = std::fs::read_to_string(&path).unwrap();
        assert!(s.contains("a.s([])"), "{chunk}: {s}");
        std::fs::write(&path, s.replace("a.s([])", "a.s([\"changed\"])")).unwrap();
    }

    fn replace_everywhere(&self, from: &str, to: &str) -> usize {
        self.edit("", from, from, to)
    }

    fn fingerprint(&self) -> Fingerprint {
        let b = NextBuild::load_dist(&self.project, &self.dist).unwrap();
        fingerprint(&b, &FingerprintOptions::default())
    }
}

fn base_fingerprint() -> Fingerprint {
    fingerprint(&load(), &FingerprintOptions::default())
}

#[test]
fn fingerprint_covers_every_route() {
    let fp = base_fingerprint();
    assert_eq!(fp.routes.len(), 15);
    let about = &fp.routes["/about"];
    assert!(
        about.server.files > 5 && about.client.files > 3,
        "{about:?}"
    );
    assert_eq!(about.css.files, 2);
    assert!(about.prerender.files >= 3);
    assert_eq!(about.own, about.effective);
    // Routes differ from each other; handlers have no client side.
    assert_ne!(about.own, fp.routes["/pricing"].own);
    assert_eq!(fp.routes["/api/hello"].client.files, 0);
    for k in [
        "middleware",
        "middleware_matchers",
        "routing",
        "route_table",
        "config",
        "images",
    ] {
        assert!(fp.global.contains_key(k), "{k}");
    }
    // The stylesheet rule sets are kept once per distinct set of files.
    assert!(!fp.css.is_empty() && fp.css.len() <= 3);
    assert!(fp.css[about.css_rules.as_ref().unwrap()].0.len() > 10);
    // Round trip.
    let json = serde_json::to_string(&fp).unwrap();
    assert_eq!(serde_json::from_str::<Fingerprint>(&json).unwrap(), fp);
}

#[test]
fn rebuild_noise_does_not_change_fingerprints() {
    // The same build elsewhere on disk, with another build id, other action ids and
    // another content-hash name for a client chunk.
    let c = Copy::new();
    let base = base_fingerprint();
    let b = load();
    assert!(c.replace_everywhere(BUILD_ID, "Zk9-qqqqqqqqqqqqqqqq1") > 0);
    // The id also occurs regex-escaped (data route regexes), here inside JSON.
    c.replace_everywhere("Al\\\\-SShdjXUxDRuupQCSZm", "Zk9\\\\-qqqqqqqqqqqqqqqq1");
    std::fs::rename(
        c.dist.join("static").join(BUILD_ID),
        c.dist.join("static/Zk9-qqqqqqqqqqqqqqqq1"),
    )
    .unwrap();
    for a in &b.actions {
        let other: String = a.id.chars().rev().collect();
        assert!(c.replace_everywhere(&a.id, &other) > 0);
    }
    let chunk = b
        .routes
        .iter()
        .find(|r| r.pathname == "/about")
        .unwrap()
        .client_files
        .iter()
        .find(|f| f.ends_with(".js") && !b.app_bootstrap.contains(f))
        .unwrap()
        .clone();
    let name = chunk.rsplit('/').next().unwrap().trim_end_matches(".js");
    assert!(c.replace_everywhere(name, "0zz0zz0zz0zz0") > 0);
    std::fs::rename(
        c.dist.join(&chunk),
        c.dist.join("static/chunks/0zz0zz0zz0zz0.js"),
    )
    .unwrap();

    let head = c.fingerprint();
    assert_ne!(head.build_id, base.build_id);
    let d = diff(&base, &head, CssMode::Strict).unwrap();
    assert!(d.is_empty(), "{d:?}");
    assert!(d.rules.added.is_empty() && d.route_table.added.is_empty());
}

#[test]
fn a_changed_module_marks_the_routes_that_load_it() {
    let c = Copy::new();
    assert!(
        c.edit(
            "server/chunks",
            "About card",
            "About card",
            "About the card"
        ) >= 1
    );
    let d = diff(&base_fingerprint(), &c.fingerprint(), CssMode::Strict).unwrap();
    assert_eq!(d.changed.keys().collect::<Vec<_>>(), vec!["/about"]);
    assert_eq!(d.changed["/about"], vec!["server"]);
    assert!(d.global.is_empty() && d.added.is_empty() && d.removed.is_empty());
}

#[test]
fn stylesheet_changes_by_rule() {
    // A new utility class in the global stylesheet: no route changes by itself.
    let c = Copy::new();
    assert_eq!(
        c.edit(
            "static/chunks",
            ".text-xl{",
            ".text-xl{",
            ".zz-new{color:red}.text-xl{"
        ),
        1
    );
    let (base, head) = (base_fingerprint(), c.fingerprint());
    let d = diff(&base, &head, CssMode::Utilities).unwrap();
    assert!(d.changed.is_empty(), "{:?}", d.changed);
    assert!(d.css_only_utilities.contains(&"/about".to_string()));
    let v = d.css.values().next().unwrap();
    assert!(v.utilities_only, "{v:?}");
    assert_eq!(v.added, vec![".zz-new"]);
    // Strictly, every page that loads the stylesheet changed.
    let strict = diff(&base, &head, CssMode::Strict).unwrap();
    assert!(strict.changed.len() >= 5);
    assert!(
        strict
            .changed
            .values()
            .all(|w| w == &vec!["css".to_string()])
    );
    assert!(!strict.changed.contains_key("/api/hello"));

    // A changed declaration of an existing rule affects every route in both modes.
    let c = Copy::new();
    assert_eq!(
        c.edit(
            "static/chunks",
            ".legacy-box{",
            ".legacy-box{",
            ".legacy-box{outline:0;"
        ),
        1
    );
    let d = diff(&base, &c.fingerprint(), CssMode::Utilities).unwrap();
    assert!(d.changed.contains_key("/about") && d.changed.contains_key("/pricing"));
}

#[test]
fn routing_changes_are_itemized() {
    let c = Copy::new();
    let manifest = c.dist.join("routes-manifest.json");
    let s = std::fs::read_to_string(&manifest).unwrap();
    assert!(s.contains("\"/old-about\""));
    std::fs::write(&manifest, s.replace("\"/old-about\"", "\"/older-about\"")).unwrap();
    let d = diff(&base_fingerprint(), &c.fingerprint(), CssMode::Strict).unwrap();
    assert!(d.changed.is_empty());
    assert_eq!(d.global, vec!["routing"]);
    assert_eq!(d.rules.added.len(), 1);
    assert_eq!(d.rules.added[0].source, "/older-about");
    assert_eq!(d.rules.removed[0].source, "/old-about");
}

fn relation() -> InvokeRelation {
    let mut r = manifest_relation(&load());
    let inv = |kind, target: &str| Invocation {
        kind,
        target: InvokeTarget::Route(target.to_string()),
        found: Found::Static,
        file: "app/x.tsx".into(),
        line: 3,
        method: None,
        via: vec!["fetch".into()],
    };
    // /pricing fetches /api/report-data, which fetches /api/hello; /about only
    // links.
    r.routes
        .entry("app/(marketing)/pricing/page".into())
        .or_default()
        .push(inv(InvokeKind::Fetch, "app/api/report-data/route"));
    r.routes
        .entry("app/api/report-data/route".into())
        .or_default()
        .push(inv(InvokeKind::Fetch, "/api/hello"));
    r.routes
        .entry("app/about/page".into())
        .or_default()
        .push(inv(InvokeKind::Navigation, "app/api/hello/route"));
    r
}

#[test]
fn effective_fingerprints_follow_invocations() {
    let mut base = base_fingerprint();
    let report = apply(&mut base, &relation());
    assert!(report.unknown_targets.is_empty(), "{report:?}");
    // The trimmed fixture lists an action of a route it does not contain.
    assert_eq!(report.unknown_routes, vec!["app/idioms/page"]);
    let pricing = &base.routes["/pricing"];
    assert_ne!(pricing.own, pricing.effective);
    assert_eq!(base.routes["/about"].own, base.routes["/about"].effective);
    assert_eq!(
        base.routes["/api/hello"].own,
        base.routes["/api/hello"].effective
    );
    // The server action is listed on the entry that bundles it, from the manifest.
    assert!(
        base.routes["/dashboard/settings"]
            .invokes
            .iter()
            .any(|i| i.kind == InvokeKind::Action && i.found == Found::Manifest)
    );

    // The innermost handler changes: its callers are affected, transitively; a
    // linking page is not.
    let c = Copy::new();
    c.change_own_chunk("/api/hello");
    let mut head = c.fingerprint();
    apply(&mut head, &relation());
    let d = diff(&base, &head, CssMode::Strict).unwrap();
    assert_eq!(d.changed.keys().collect::<Vec<_>>(), vec!["/api/hello"]);
    let affected: BTreeSet<&String> = d.affected.keys().collect();
    assert_eq!(
        affected.into_iter().collect::<Vec<_>>(),
        vec!["/api/report-data", "/pricing"]
    );
    assert_eq!(d.affected["/pricing"], vec!["/api/hello"]);
    assert_eq!(head.routes["/pricing"].own, base.routes["/pricing"].own);
    assert_ne!(
        head.routes["/pricing"].effective,
        base.routes["/pricing"].effective
    );

    // An unevaluable request stands for every handler.
    let mut open = InvokeRelation::new();
    open.routes.insert(
        "/about".into(),
        vec![Invocation {
            kind: InvokeKind::Fetch,
            target: InvokeTarget::Unknown("computed URL".into()),
            found: Found::Static,
            file: String::new(),
            line: 0,
            method: None,
            via: Vec::new(),
        }],
    );
    let (mut b2, mut h2) = (base_fingerprint(), c.fingerprint());
    assert_eq!(apply(&mut b2, &open).open_routes, vec!["/about"]);
    apply(&mut h2, &open);
    let d = diff(&b2, &h2, CssMode::Strict).unwrap();
    assert_eq!(d.affected.keys().collect::<Vec<_>>(), vec!["/about"]);
}

#[test]
fn volatile_values_are_not_content() {
    // The same module with another inlined value (a commit SHA through `env`).
    let c = Copy::new();
    assert!(
        c.edit(
            "server/chunks",
            "About card",
            "About card",
            "sha-1111111aaaaaaa"
        ) >= 1
    );
    let other = Copy::new();
    assert!(
        other.edit(
            "server/chunks",
            "About card",
            "About card",
            "sha-9999999ddddddd"
        ) >= 1
    );
    let fp = |c: &Copy, value: &str| {
        let b = NextBuild::load_dist(&c.project, &c.dist).unwrap();
        fingerprint(
            &b,
            &FingerprintOptions {
                volatile: vec![("COMMIT".into(), value.into())],
                ..FingerprintOptions::default()
            },
        )
    };
    let d = diff(
        &fp(&c, "sha-1111111aaaaaaa"),
        &fp(&other, "sha-9999999ddddddd"),
        CssMode::Strict,
    )
    .unwrap();
    assert!(d.is_empty(), "{d:?}");
    // Without declaring it, the route changes.
    let d = diff(&c.fingerprint(), &other.fingerprint(), CssMode::Strict).unwrap();
    assert_eq!(d.changed.keys().collect::<Vec<_>>(), vec!["/about"]);
}
