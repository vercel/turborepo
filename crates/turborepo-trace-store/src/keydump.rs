//! Diagnostics: what a first-party key was computed from, item by item.
//!
//! With `TURBO_TRACE_DEBUG_KEY_DUMP=<file>` a build appends one tab-separated
//! line per input of every first-party key it computes:
//!
//! ```text
//! <phase> \t <package root> \t <component> \t <item> \t <value>
//! ```
//!
//! `phase` is `validate` (the previous shard's observations against the current
//! tree: what decides whether the shard is reused) or `assemble` (a shard
//! traced by this build). Two dumps of one shard (the build that traced it, a
//! later refresh that validated it) differ in exactly the inputs that re-keyed
//! it.
//!
//! The values are digests and states, never file contents.

use std::{
    collections::BTreeSet,
    io::Write,
    path::Path,
    sync::{Mutex, OnceLock},
};

use crate::{
    build::{Cx, LOCK_LINK},
    normalize::{ABOVE, PRESENCE},
    paths::{abs, rel_to},
    shard::ArchivedShard,
    workspace::PackageDef,
};

fn sink() -> Option<&'static Mutex<std::fs::File>> {
    static SINK: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
    SINK.get_or_init(|| {
        let p = turborepo_trace_core::debug::var_os("KEY_DUMP")?;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .ok()
            .map(Mutex::new)
    })
    .as_ref()
}

/// Whether a dump was asked for.
pub(crate) fn enabled() -> bool {
    sink().is_some()
}

/// Appends the inputs of the key of `def`'s shard `s` in the current tree.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dump(
    cx: &Cx,
    tasks: &crate::tasks::Tasks<'_>,
    phase: &str,
    def: &PackageDef,
    manifest_hash: Option<u128>,
    entries: &[(String, Option<u128>)],
    s: &ArchivedShard,
    key: &str,
    content: Option<u128>,
) {
    let Some(sink) = sink() else {
        return;
    };
    let fs = cx.tracer.fs();
    let root = cx.repo.join(&def.root);
    let root = if def.root.is_empty() {
        cx.repo.clone()
    } else {
        root
    };
    let mut out = String::new();
    let mut line = |component: &str, item: &str, value: &str| {
        out.push_str(phase);
        out.push('\t');
        out.push_str(if def.root.is_empty() { "." } else { &def.root });
        out.push('\t');
        out.push_str(component);
        out.push('\t');
        out.push_str(&item.replace(['\t', '\n'], " "));
        out.push('\t');
        out.push_str(&value.replace(['\t', '\n'], " "));
        out.push('\n');
    };
    let hex = |x: u128| format!("{x:032x}");
    line("key", "", key);
    if let Some(c) = content {
        line("content", "", &hex(c));
    }
    line("options", "", &cx.opts_digest);
    line("fw-configs", "", &hex(cx.fw_configs.digest_for(&def.root)));
    // The place of the checkout, where it decides a project's tests
    // (`crate::located`).
    for l in s.located.iter() {
        line(
            "located",
            l.config.as_str(),
            &cx.places.state(l.spec.as_str()),
        );
    }
    // The place of the checkout, where a tsconfig with aliases governs
    // (`crate::fw::place_state`).
    if let Some(below) = crate::fw::place_state(cx, s) {
        line("aliased", "", crate::fw::place_word(below));
    }
    line("name", "", &def.name);
    line("manifest", "", &manifest_hash.map_or("none".into(), hex));
    for (p, x) in entries {
        line("entry", p, &x.map_or("unreadable".into(), hex));
    }
    // With TURBO_TRACE_DEBUG_KEY_DUMP_FILES: which table entries each file's
    // references read (`p<i>`, `m<i>`, `c<i>`; an `f` suffix for a framework
    // group, a `k` suffix for a set of key observations), and the entries'
    // names.
    let files = turborepo_trace_core::debug::var_os("KEY_DUMP_FILES").is_some();
    // Probes and guards: only the paths that exist enter the digest.
    let mut present: BTreeSet<(&str, String, std::borrow::Cow<'static, str>)> = BTreeSet::new();
    let mut present_ix: BTreeSet<(String, usize)> = BTreeSet::new();
    let mut paths = Vec::new();
    for (i, p) in s.probes.iter().enumerate() {
        for (tag, expand) in PRESENCE {
            paths.clear();
            expand(p, &mut paths);
            for rel in &paths {
                let a = abs(&root, rel);
                if tag == ABOVE && !a.starts_with(&cx.repo) {
                    continue;
                }
                if let Some(kind) = crate::normalize::presence(fs, &a) {
                    present.insert((tag, rel.clone(), kind));
                    if files {
                        present_ix.insert((format!("{tag} {rel}"), i));
                    }
                }
            }
        }
    }
    for (tag, rel, kind) in &present {
        line(tag, rel, kind);
    }
    for rel in crate::normalize::probe_dirs(s.probes.as_slice()) {
        if fs.is_dir(&abs(&root, &rel)) {
            line("probe-dir-present", &rel, "present");
        } else {
            let nearest = crate::normalize::nearest_dir_on(fs, &root, &rel);
            line(crate::normalize::DIR_ABSENT, &rel, &nearest);
        }
    }
    for (what, i) in &present_ix {
        line("ix", what, &format!("p{i}"));
    }
    if files {
        for (i, m) in s.memberships.iter().enumerate() {
            line(
                "ix",
                &format!("member {} :: {}", m.dir.as_str(), m.pattern.as_str()),
                &format!("m{i}"),
            );
        }
        for (i, c) in s.configs.iter().enumerate() {
            let shown = rel_to(&cx.repo, &cx.repo, &abs(&root, c.as_str()));
            line("ix", &format!("config {shown}"), &format!("c{i}"));
        }
        for f in s.files.iter() {
            if crate::tasks::is_pseudo_lang(f.lang.as_ref().map(|l| l.as_str())) {
                continue;
            }
            let mut toks: BTreeSet<String> = BTreeSet::new();
            for r in f.refs.iter() {
                let fw = if r.index.to_native() == crate::shard::FRAMEWORK_LEVEL {
                    "f"
                } else {
                    ""
                };
                toks.extend(r.probes.iter().map(|i| format!("p{}{fw}", i.to_native())));
                toks.extend(
                    r.memberships
                        .iter()
                        .map(|i| format!("m{}{fw}", i.to_native())),
                );
                toks.extend(r.configs.iter().map(|i| format!("c{}{fw}", i.to_native())));
            }
            // The entries of the file's sets of key observations: a `k` suffix.
            for g in f.key_groups.iter() {
                let g = &s.key_groups[g.to_native() as usize];
                toks.extend(g.probes.iter().map(|i| format!("p{}k", i.to_native())));
                toks.extend(g.memberships.iter().map(|i| format!("m{}k", i.to_native())));
                toks.extend(g.configs.iter().map(|i| format!("c{}k", i.to_native())));
            }
            let toks: Vec<String> = toks.into_iter().collect();
            line("file-refs", f.path.as_str(), &toks.join(" "));
        }
    }
    // Who reads each membership: the languages of the referencing files, and the
    // pseudo-files by name.
    let mut readers: Vec<BTreeSet<String>> = vec![BTreeSet::new(); s.memberships.len()];
    for f in s.files.iter() {
        let who = match f.lang.as_ref().map(|l| l.as_str()) {
            Some(l) if crate::tasks::is_pseudo_lang(Some(l)) => {
                let p = f.path.as_str();
                format!("{l}:{}", &p[..p.len().min(60)])
            }
            Some(l) => l.to_owned(),
            None => "-".to_owned(),
        };
        for r in f.refs.iter() {
            let group = match r.index.to_native() {
                crate::shard::FILE_LEVEL => "/file",
                crate::shard::FRAMEWORK_LEVEL => "/framework",
                _ => "",
            };
            for m in r.memberships.iter() {
                let set = &mut readers[m.to_native() as usize];
                if set.len() < 6 {
                    set.insert(format!("{who}{group}"));
                }
            }
        }
    }
    for hub in s.hubs.iter() {
        let exclude: Vec<String> = hub.exclude.iter().map(|e| e.to_string()).collect();
        let d = tasks.hub_state(hub.dir.as_str(), hub.pattern.as_str(), &exclude, hub.own);
        line(
            "hub",
            &format!(
                "{} :: {} !{} {}",
                hub.dir.as_str(),
                hub.pattern.as_str(),
                exclude.join(" !"),
                if hub.own { "(own files)" } else { "" }
            ),
            &hex(d),
        );
    }
    for (i, m) in s.memberships.iter().enumerate() {
        let item = format!("{} :: {}", m.dir.as_str(), m.pattern.as_str());
        // A listing only hubs reference is keyed by the hubs' states.
        if s.hub_members.binary_search(&(i as u32).into()).is_ok() {
            line("member", &item, "hub-only");
        } else if s.key_only.binary_search(&(i as u32).into()).is_ok() {
            let (d, _) = cx.membership_digest(&abs(&root, m.dir.as_str()), m.pattern.as_str());
            line("key-member", &item, &hex(d));
        } else {
            let (d, _) = cx.membership_digest(&abs(&root, m.dir.as_str()), m.pattern.as_str());
            line("member", &item, &hex(d));
        }
        let who: Vec<&str> = readers[i].iter().map(String::as_str).collect();
        line(
            "member-readers",
            &item,
            &if who.is_empty() {
                "key-only".to_owned()
            } else {
                who.join(" | ")
            },
        );
    }
    for f in s.obs.found.iter() {
        let p = abs(&root, f.as_str());
        let owner = p
            .strip_prefix(&cx.repo)
            .ok()
            .map(|rel| {
                cx.ws.packages[cx.ws.owner(&rel.to_string_lossy())]
                    .name
                    .clone()
            })
            .unwrap_or_default();
        line(
            "found",
            f.as_str(),
            &format!("{} {owner}", if fs.is_file(&p) { "file" } else { "absent" }),
        );
    }
    let path_only = |i: usize| s.path_configs.binary_search(&(i as u32).into()).is_ok();
    for (i, c) in s.configs.iter().enumerate() {
        let p = abs(&root, crate::normalize::config_path(c.as_str()));
        // Repo-relative, so one config read by many packages is one item.
        let shown = rel_to(&cx.repo, &cx.repo, &p);
        if crate::normalize::view_config(c.as_str()).is_some() {
            line(
                // (The resolution view of a manifest, or a config loader's.)
                if crate::normalize::loader_config(c.as_str()).is_some() {
                    "config-loader"
                } else {
                    "config-view"
                },
                &shown,
                &crate::build::view_digest(cx, c.as_str(), &p).map_or("missing".into(), hex),
            );
        } else if path_only(i) {
            line(
                "config-path",
                &shown,
                if fs.is_file(&p) { "file" } else { "missing" },
            );
        } else {
            line(
                "config",
                &shown,
                &cx.hashes.hash(&p).ok().map_or("missing".into(), hex),
            );
        }
    }
    for r in s.dep_closures.iter() {
        let v = match cx.task_graph.closure(r.as_str()) {
            Some(ix) => ix
                .into_iter()
                .map(|i| cx.task_graph.ws.packages[i].rel.clone())
                .collect::<Vec<_>>()
                .join(","),
            None => "absent".into(),
        };
        line(
            "closure",
            r.as_str(),
            &hex(xxhash_rust::xxh3::xxh3_128(v.as_bytes())),
        );
    }
    for l in s.obs.links.iter() {
        if let Some(rest) = l.location.strip_prefix(LOCK_LINK) {
            let (importer, name) = rest.split_once(':').unwrap_or((rest, ""));
            let v = match cx.ids.lock_resolve(importer, name) {
                Some(crate::identity::LockTarget::Package(id)) => {
                    format!("{}@{} {}", id.name, id.version, id.integrity.is_some())
                }
                Some(crate::identity::LockTarget::Workspace(r)) => format!("workspace {r}"),
                None => "absent".into(),
            };
            line("lock-link", l.location.as_str(), &v);
            continue;
        }
        if let Some(rest) = l.location.strip_prefix(crate::build::SEEN_LINK) {
            let loc = abs(&root, rest);
            line(
                "seen-link",
                &rel_to(&cx.repo, &cx.repo, &loc),
                &hex(crate::build::seen_digest(cx, &loc)),
            );
            continue;
        }
        let loc = abs(&root, l.location.as_str());
        let v = if let Some(li) = cx.ids.language(&loc, fs) {
            format!(
                "language {}@{} {}",
                li.id.name,
                li.id.version,
                rel_to(&cx.repo, &cx.repo, &li.root)
            )
        } else {
            match crate::build::link_identity(cx, &loc) {
                Some((id, real)) => format!(
                    "{}@{} {}",
                    id.name,
                    id.version,
                    rel_to(&cx.repo, &cx.repo, &real)
                ),
                None => "absent".into(),
            }
        };
        line("link", &rel_to(&cx.repo, &cx.repo, Path::new(&loc)), &v);
    }
    let mut f = sink.lock().unwrap_or_else(|e| e.into_inner());
    let _ = f.write_all(out.as_bytes());
}
