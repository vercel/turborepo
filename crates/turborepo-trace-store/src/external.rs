//! Links to language packages that are known from a manifest but not installed.
//!
//! A Go import of a package that `go.mod` requires resolves into the module
//! cache. Without a module cache (a fresh CI checkout, a sandbox) the resolver
//! still knows which module version provides the package: it reports the edge
//! as [`turborepo_trace_core::Target::External`] `module@version` with an
//! `unresolved_specifier` record. Here that edge becomes what it would be with
//! the module downloaded: a link to the opaque package `go:<module>@<version>`
//! ([`LinkRec`] at the module's directory in the cache, [`CrossRec`] into it),
//! and the record is dropped.
//!
//! Why the record can go: what such an import compiles against is decided by
//! the build list, that is by `go.mod` (and `go.work`, `vendor/modules.txt`)
//! and verified by `go.sum`. Those files are manifest edges of every Go file of
//! the module and config inputs of the resolution, the version pins of the
//! link, and the lookups in the module cache are probes. So every change that
//! can alter the import seeds the importing file, and a module cache
//! that appears later re-resolves it to the same package node. The graph is the
//! same with and without the cache.
//!
//! The link's location is the module's would-be directory, so it is validated
//! like the link of an installed module
//! ([`crate::identity::Identities::language`] on the location).

use std::path::PathBuf;

use turborepo_trace_core::{Edge, RefFlags, RefKind, Target};

use crate::{
    build::TargetCx,
    paths::rel_to,
    shard::{CrossRec, LinkRec},
};

/// The flags a file read by Python, Go or Rust code adds to its edge: the
/// reader observes the target's contents and does not run it, so the edge is a
/// data edge ([`RefFlags::DATA`]). A change of the target still reaches the
/// reader; what the target would load if it were run (a fixture that happens to
/// be source code: its unresolved imports, its own computed reads) is not the
/// reader's uncertainty. Code that runs what it read does so through an
/// `eval`-like call, which has its own record.
///
/// An embed (`include_str!`, `//go:embed`) is a read of contents too, unless
/// what it embeds is source of the embedding language
/// (`include!("generated.rs")` is compiled in): `targets` are the embedded
/// files.
///
/// JS/TS edges keep their flags until the same rule has been validated on the
/// JS corpora.
pub(crate) fn read_flags<'p>(
    lang: Option<&str>,
    kind: RefKind,
    mut targets: impl Iterator<Item = &'p std::path::Path>,
) -> RefFlags {
    let exts: &[&str] = match lang {
        Some("python") => &["py", "pyi", "pyw"],
        Some("go") => &["go"],
        Some("rust") => &["rs"],
        _ => return RefFlags::empty(),
    };
    let data = match kind {
        RefKind::FsRead | RefKind::FsReadDir | RefKind::Asset => true,
        RefKind::Embed => !targets.any(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| exts.contains(&e))
        }),
        _ => false,
    };
    if data {
        RefFlags::DATA
    } else {
        RefFlags::empty()
    }
}

/// The link for an edge whose target is an external package name a language can
/// identify ([`turborepo_trace_core::Resolver::external_identity`]): the link
/// record, the edge target and the files that pin the version. `None` for every
/// other edge, and in traced third-party shards (their languages' packages are
/// followed, as in [`TargetCx`]'s classification).
pub(crate) fn link(tcx: &TargetCx<'_>, edge: &Edge) -> Option<(LinkRec, CrossRec, Vec<PathBuf>)> {
    let Target::External(name) = &edge.to else {
        return None;
    };
    if tcx.follow {
        return None;
    }
    let cx = tcx.cx;
    let li = cx.ids.external(name, cx.tracer.fs())?;
    let link = LinkRec {
        location: rel_to(tcx.root_abs, &cx.repo, &li.root),
        name: li.id.name.clone(),
        version: li.id.version.clone(),
        integrity: li.id.integrity.clone(),
        root: rel_to(&cx.repo, &cx.repo, &li.root),
    };
    // The package directory inside the module, for display: the import path below
    // the module path (`github.com/a/b/sub` in `github.com/a/b@v1` is `sub`).
    let module = name.rsplit_once('@').map_or(&**name, |(m, _)| m);
    let subpath = edge
        .spec
        .strip_prefix(module)
        .map(|s| s.trim_start_matches('/').to_owned())
        .unwrap_or_default();
    let cross = CrossRec {
        name: li.id.name,
        version: Some(li.id.version),
        subpath,
    };
    Some((link, cross, li.pins))
}
