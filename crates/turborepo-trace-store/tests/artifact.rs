//! Graph artifacts (`turborepo_trace_store::artifact`): a graph packed at one
//! checkout, unpacked at another and refreshed is byte-identical (JSON export
//! and shard keys) to a fresh build there, whatever was edited in between; the
//! artifact's bytes are a function of the tree; and an artifact that does not
//! fit installs nothing.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use proptest::prelude::*;
use turborepo_trace_core::Language;
use turborepo_trace_store::{
    Externals, Graph, GraphOptions,
    artifact::{self, ArtifactError, Header, PackOptions},
    build,
};

fn langs() -> Vec<Arc<dyn Language>> {
    vec![Arc::new(turborepo_trace_lang_js::JsLanguage)]
}

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, contents).unwrap();
}

fn symlink(root: &Path, target: &str, link: &str) {
    let l = root.join(link);
    std::fs::create_dir_all(l.parent().unwrap()).unwrap();
    let _ = std::fs::remove_file(&l);
    turborepo_trace_core::os::symlink(target, l).unwrap();
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap()
}

/// Installs lodash `version` pnpm-style and writes the lockfile.
fn lodash(r: &Path, version: &str, integrity: &str) {
    let dir = format!("node_modules/.pnpm/lodash@{version}/node_modules/lodash");
    write(
        r,
        &format!("{dir}/package.json"),
        &format!(r#"{{"name":"lodash","version":"{version}","main":"index.js"}}"#),
    );
    write(r, &format!("{dir}/index.js"), "module.exports = 1;\n");
    symlink(
        r,
        &format!("../../../node_modules/.pnpm/lodash@{version}/node_modules/lodash"),
        "packages/a/node_modules/lodash",
    );
    write(
        r,
        "pnpm-lock.yaml",
        &format!(
            "lockfileVersion: '9.0'\nimporters:\n  .: {{}}\n  packages/a:\n    dependencies:\n      lodash:\n        specifier: ^4\n        version: {version}\npackages:\n  lodash@{version}:\n    resolution: {{integrity: {integrity}}}\nsnapshots:\n  lodash@{version}: {{}}\n"
        ),
    );
}

/// A workspace with packages, a pnpm-style install, a lockfile, tasks with
/// outputs, membership hubs and files that are no module.
fn fixture(r: &Path) {
    write(r, ".gitignore", "node_modules\n.turbotrace\n");
    write(
        r,
        "package.json",
        r#"{"name":"root","private":true,"workspaces":["packages/*"],"scripts":{"gen":"node gen.js"}}"#,
    );
    write(r, "gen.js", "console.log(1);\n");
    write(
        r,
        "turbo.json",
        r#"{"tasks":{"//#gen":{"outputs":["tools/**/*.gen.ts"]},"build":{"dependsOn":["^build"]}}}"#,
    );
    write(
        r,
        "tools/b.js",
        "const fs = require('fs');\nconst path = require('path');\nmodule.exports = \
         fs.readdirSync(path.join(__dirname, 'fixtures'));\nrequire('./x.gen');\n",
    );
    write(r, "tools/fixtures/g.json", "{}\n");
    write(r, "src/root.ts", "import a from 'a';\nexport default a;\n");
    write(
        r,
        "packages/a/package.json",
        r#"{"name":"a","main":"src/index.ts","scripts":{"build":"tsc -p ."}}"#,
    );
    write(
        r,
        "packages/a/src/index.ts",
        "import { u } from './util';\nimport b from 'b';\nimport _ from 'lodash';\nimport \
         './missing';\nconst loc = require('./locales/' + lang + '.json');\nexport const x = u;\n",
    );
    write(r, "packages/a/src/util.ts", "export const u = 1;\n");
    write(r, "packages/a/src/locales/en.json", "{}");
    write(
        r,
        "packages/b/package.json",
        r#"{"name":"b","main":"src/index.ts","scripts":{"build":"tsc -p ."}}"#,
    );
    write(r, "packages/b/src/index.ts", "export default 2;\n");
    write(r, "docs/guide.md", "# guide\n");
    lodash(r, "4.17.21", "sha512-old");
    symlink(r, "../packages/a", "node_modules/a");
    symlink(r, "../../../packages/b", "packages/a/node_modules/b");
}

fn opts(root: &Path, out: &Path) -> GraphOptions {
    GraphOptions {
        root: root.to_path_buf(),
        out: Some(out.to_path_buf()),
        externals: Externals::Opaque,
        threads: 2,
        ..Default::default()
    }
}

/// The JSON export and the shard keys of the graph in `out`.
fn export(out: &Path) -> (String, Vec<String>) {
    let g = Graph::open(out).unwrap();
    let mut buf = Vec::new();
    turborepo_trace_graph::json::write_json(&g, Some(g.root()), true, &mut buf).unwrap();
    let keys = g
        .manifest()
        .shards
        .iter()
        .map(|s| format!("{} {} {}", s.kind, s.root, s.key))
        .collect();
    (String::from_utf8(buf).unwrap(), keys)
}

/// The primary store of the graph in `out` as bytes: its packed file. Equal for
/// two graphs exactly when their manifests, walked-file lists and every shard's
/// bytes are equal.
fn store_bytes(out: &Path) -> Vec<u8> {
    let t = tempfile::tempdir().unwrap();
    let file = t.path().join("store.tta");
    artifact::pack(
        out,
        &file,
        &PackOptions {
            label: Some("same".into()),
            ..Default::default()
        },
    )
    .unwrap();
    std::fs::read(file).unwrap()
}

/// Copies a tree (files, directories, symbolic links as links) without its
/// graph directory.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        if e.file_name() == ".turbotrace" {
            continue;
        }
        let t = to.join(e.file_name());
        let ty = e.file_type().unwrap();
        if ty.is_symlink() {
            turborepo_trace_core::os::symlink(std::fs::read_link(e.path()).unwrap(), &t).unwrap();
        } else if ty.is_dir() {
            copy_tree(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), &t).unwrap();
        }
    }
}

/// What the CLI passes to `unpack`: the digest this binary computes for the
/// artifact's options at `root`.
fn expect<'a>(root: &'a Path, out: &'a Path) -> impl Fn(&Header) -> Result<String, String> + 'a {
    move |_| {
        turborepo_trace_store::build::options_digest_of(&opts(root, out), &langs())
            .map_err(|e| e.to_string())
    }
}

fn unpack(file: &Path, root: &Path, out: &Path) -> Result<artifact::UnpackReport, ArtifactError> {
    artifact::unpack(file, out, root, &expect(root, out), &[], false)
}

/// A repository with a built graph, and a place for artifacts.
struct Packed {
    _t: tempfile::TempDir,
    root: PathBuf,
    scratch: PathBuf,
}

impl Packed {
    fn new(make: impl FnOnce(&Path)) -> Packed {
        let t = tempfile::tempdir().unwrap();
        let base = canonical(t.path());
        let root = base.join("first/checkout");
        std::fs::create_dir_all(&root).unwrap();
        make(&root);
        build(&opts(&root, &root.join(".turbotrace")), &langs()).unwrap();
        let scratch = base.join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        Packed {
            _t: t,
            root,
            scratch,
        }
    }

    fn pack(&self, name: &str, caches: bool) -> PathBuf {
        let file = self.scratch.join(name);
        artifact::pack(
            &self.root.join(".turbotrace"),
            &file,
            &PackOptions {
                label: Some("0123abcd".into()),
                caches,
                ..Default::default()
            },
        )
        .unwrap();
        file
    }

    /// The same tree at another location, without a graph.
    fn elsewhere(&self, name: &str) -> PathBuf {
        let to = self.scratch.join(name).join("deeper/other-checkout");
        copy_tree(&self.root, &to);
        to
    }
}

/// Refreshes the restored graph of `root` and compares it with a fresh build of
/// the tree.
fn refreshed_equals_cold(root: &Path, what: &str) -> turborepo_trace_store::BuildReport {
    let out = root.join(".turbotrace");
    let r = build(&opts(root, &out), &langs()).unwrap();
    let fresh = tempfile::tempdir().unwrap();
    build(&opts(root, fresh.path()), &langs()).unwrap();
    let (got_json, got_keys) = export(&out);
    let (fresh_json, fresh_keys) = export(fresh.path());
    if got_json != fresh_json {
        let got: std::collections::BTreeSet<&str> = got_json.lines().collect();
        let fr: std::collections::BTreeSet<&str> = fresh_json.lines().collect();
        let mut diff: Vec<String> = got
            .difference(&fr)
            .take(6)
            .map(|l| format!("only restored: {}", l.trim()))
            .collect();
        diff.extend(
            fr.difference(&got)
                .take(6)
                .map(|l| format!("only fresh: {}", l.trim())),
        );
        panic!(
            "{what}: the restored graph differs from a fresh build:\n{}",
            diff.join("\n")
        );
    }
    assert_eq!(got_keys, fresh_keys, "{what}: shard keys");
    // Same keys, same bytes: what a store keeps under a key is what a fresh build
    // writes.
    assert!(
        store_bytes(&out) == store_bytes(fresh.path()),
        "{what}: the stores differ in bytes under equal keys"
    );
    r
}

#[test]
fn a_graph_restored_at_another_location_is_reused_whole() {
    let p = Packed::new(fixture);
    let file = p.pack("graph.tta", false);
    let h = artifact::read_header(&file).unwrap();
    assert_eq!(h.label.as_deref(), Some("0123abcd"));
    assert_eq!(h.build_id, turborepo_trace_store::build::build_id());
    assert!(!h.has_caches());
    assert_eq!(
        h.sections
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["graph.json", "files", "pack"]
    );

    let there = p.elsewhere("b");
    let out = there.join(".turbotrace");
    let r = unpack(&file, &there, &out).unwrap();
    assert_eq!(r.header, h);
    // Only primary files: nothing keyed by the first checkout's file stamps.
    assert!(!out.join("cache").exists());
    assert!(!out.join("store/pack.log").exists());
    let m = turborepo_trace_store::Manifest::read(&out).unwrap();
    assert_eq!(Path::new(&m.root), there);

    let r = refreshed_equals_cold(&there, "same tree, another location");
    assert_eq!((r.traced, r.written), (0, 0), "every shard is reused");
    assert_eq!(r.analyzed_files, 0);
    assert!(r.reused > 2);
}

#[test]
fn an_edit_after_the_artifact_retraces_its_package_only() {
    let p = Packed::new(fixture);
    let file = p.pack("graph.tta", false);
    let there = p.elsewhere("b");
    write(&there, "packages/b/src/index.ts", "export default 3;\n");
    write(&there, "packages/b/src/more.ts", "import './index';\n");
    unpack(&file, &there, &there.join(".turbotrace")).unwrap();
    let r = refreshed_equals_cold(&there, "one package edited");
    assert_eq!(r.traced, 1, "only packages/b");
    assert_eq!(r.analyzed_files, 2);
}

#[test]
fn the_artifact_is_a_function_of_the_tree() {
    let p = Packed::new(fixture);
    let first = std::fs::read(p.pack("one.tta", false)).unwrap();
    let first_caches = std::fs::read(p.pack("one-caches.tta", true)).unwrap();
    // A no-op refresh and another pack: the same bytes.
    build(&opts(&p.root, &p.root.join(".turbotrace")), &langs()).unwrap();
    assert_eq!(first, std::fs::read(p.pack("again.tta", false)).unwrap());

    // The same tree somewhere else, built with another thread count.
    let there = p.elsewhere("c");
    let out = there.join(".turbotrace");
    build(
        &GraphOptions {
            threads: 5,
            ..opts(&there, &out)
        },
        &langs(),
    )
    .unwrap();
    for (caches, expected) in [(false, &first), (true, &first_caches)] {
        let file = p.scratch.join("two.tta");
        artifact::pack(
            &out,
            &file,
            &PackOptions {
                label: Some("0123abcd".into()),
                caches,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            *expected == std::fs::read(&file).unwrap(),
            "same tree, same artifact bytes (caches: {caches})"
        );
    }
}

#[test]
fn what_is_checked_against_its_graph_travels_when_it_is_there() {
    let p = Packed::new(fixture);
    let plain = std::fs::read(p.pack("plain.tta", false)).unwrap();
    // A query's recorded binding run, as far as the store is concerned: bytes it
    // carries.
    let name = artifact::CHECKED[0];
    assert_eq!(name, "cache/xfile-trace");
    let kept = p.root.join(".turbotrace").join(name);
    std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
    std::fs::write(&kept, b"a recorded run").unwrap();
    let file = p.pack("graph.tta", false);
    let h = artifact::read_header(&file).unwrap();
    assert!(!h.has_caches());
    assert_eq!(
        h.sections
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["graph.json", "files", "pack", name]
    );
    // Elsewhere, and on a tree that was edited since: installed as it is (its
    // reader checks it), next to a graph that is the graph of a fresh build.
    let there = p.elsewhere("b");
    write(&there, "packages/b/src/index.ts", "export default 3;\n");
    let out = there.join(".turbotrace");
    unpack(&file, &there, &out).unwrap();
    assert_eq!(std::fs::read(out.join(name)).unwrap(), b"a recorded run");
    assert!(!out.join("cache/composed").exists());
    // (The comparison packs both stores: without the file, which only this one
    // has.)
    std::fs::remove_file(out.join(name)).unwrap();
    refreshed_equals_cold(&there, "restored with a recorded run, then edited");
    // An artifact without one leaves the one of a graph it replaces where it is.
    std::fs::write(out.join(name), b"another run").unwrap();
    let plain_file = p.scratch.join("plain.tta");
    artifact::unpack(&plain_file, &out, &there, &expect(&there, &out), &[], true).unwrap();
    assert_eq!(std::fs::read(out.join(name)).unwrap(), b"another run");
    // An empty file is nothing to carry.
    std::fs::write(&kept, b"").unwrap();
    assert!(plain == std::fs::read(p.pack("empty.tta", false)).unwrap());
    // A damaged section installs nothing.
    let mut bytes = std::fs::read(&file).unwrap();
    let n = bytes.len();
    bytes[n - 1] ^= 0x55;
    let bad = p.scratch.join("bad.tta");
    std::fs::write(&bad, &bytes).unwrap();
    let again = p.elsewhere("c");
    assert!(unpack(&bad, &again, &again.join(".turbotrace")).is_err());
    assert!(!again.join(".turbotrace").join(name).exists());
    assert!(turborepo_trace_store::Manifest::read(&again.join(".turbotrace")).is_none());
}

#[test]
fn packed_caches_answer_the_first_open() {
    let p = Packed::new(fixture);
    let file = p.pack("graph.tta", true);
    let h = artifact::read_header(&file).unwrap();
    assert!(h.has_caches());
    let there = p.elsewhere("b");
    let out = there.join(".turbotrace");
    unpack(&file, &there, &out).unwrap();
    let r = refreshed_equals_cold(&there, "restored with caches");
    assert_eq!(r.traced, 0);
    // A graph restored a second time: the composed graph comes from the artifact.
    let again = p.elsewhere("d");
    let out = again.join(".turbotrace");
    unpack(&file, &again, &out).unwrap();
    build(&opts(&again, &out), &langs()).unwrap();
    let g = Graph::open(&out).unwrap();
    assert_eq!(g.load_stats().link_us, 0, "composed parts were not rebuilt");
    // After an edit the packed caches are stale and ignored.
    let edited = p.elsewhere("e");
    write(
        &edited,
        "packages/a/src/util.ts",
        "import './other';\nexport const u = 2;\n",
    );
    write(&edited, "packages/a/src/other.ts", "export {};\n");
    unpack(&file, &edited, &edited.join(".turbotrace")).unwrap();
    refreshed_equals_cold(&edited, "restored with caches, then edited");
}

/// Rewrites the header of an artifact (and its checksum).
fn with_header(bytes: &[u8], edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let n = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let mut h: serde_json::Value = serde_json::from_slice(&bytes[12..12 + n]).unwrap();
    edit(&mut h);
    let head = serde_json::to_vec(&h).unwrap();
    let mut out = bytes[..8].to_vec();
    out.extend_from_slice(&(head.len() as u32).to_le_bytes());
    out.extend_from_slice(&head);
    out.extend_from_slice(&xxhash_rust::xxh3::xxh3_64(&head).to_le_bytes());
    out.extend_from_slice(&bytes[12 + n + 8..]);
    out
}

#[test]
fn an_artifact_that_does_not_fit_installs_nothing() {
    let p = Packed::new(fixture);
    let good = std::fs::read(p.pack("graph.tta", true)).unwrap();
    let there = p.elsewhere("b");
    let out = there.join(".turbotrace");
    let file = p.scratch.join("bad.tta");
    let try_bytes = |bytes: &[u8]| {
        std::fs::write(&file, bytes).unwrap();
        let r = unpack(&file, &there, &out);
        // Nothing a build or a query would pick up.
        assert!(turborepo_trace_store::Manifest::read(&out).is_none());
        assert!(!out.join("store/pack").exists());
        assert!(!out.join("cache/composed").exists());
        r.unwrap_err()
    };

    assert!(matches!(try_bytes(b""), ArtifactError::NotAnArtifact(_)));
    assert!(matches!(
        try_bytes(b"not an artifact at all"),
        ArtifactError::NotAnArtifact(_)
    ));
    // Cut off: in the header, and in the chunks.
    assert!(matches!(try_bytes(&good[..40]), ArtifactError::Corrupt(_)));
    assert!(matches!(
        try_bytes(&good[..good.len() - 1]),
        ArtifactError::Corrupt(_)
    ));
    let mut longer = good.clone();
    longer.push(0);
    assert!(matches!(try_bytes(&longer), ArtifactError::Corrupt(_)));
    // A flipped bit in the header, and in a chunk (every bit of every byte:
    // `no_damaged_byte_installs_anything`).
    let n = u32::from_le_bytes(good[8..12].try_into().unwrap()) as usize;
    let mut bad = good.clone();
    bad[12 + n / 2] ^= 1;
    assert!(matches!(try_bytes(&bad), ArtifactError::Corrupt(_)));
    let mut bad = good.clone();
    bad[12 + n + 8 + 20] ^= 0x10;
    let e = try_bytes(&bad);
    assert!(matches!(e, ArtifactError::Corrupt(_)), "{e}");
    // Another binary, shard format, schema version.
    let e = try_bytes(&with_header(&good, |h| h["build_id"] = "0000".into()));
    assert!(matches!(e, ArtifactError::Incompatible(_)), "{e}");
    let e = try_bytes(&with_header(&good, |h| h["format"] = 1.into()));
    assert!(matches!(e, ArtifactError::Incompatible(_)), "{e}");
    let e = try_bytes(&with_header(&good, |h| h["version"] = 99.into()));
    assert!(matches!(e, ArtifactError::NotAnArtifact(_)), "{e}");
    // Options this host computes another digest for (another platform, other
    // flags).
    let e = try_bytes(&with_header(&good, |h| {
        h["options_digest"] = "00000000000000000000000000000000".into()
    }));
    assert!(matches!(e, ArtifactError::Incompatible(_)), "{e}");
    // A chunk table that does not add up.
    let e = try_bytes(&with_header(&good, |h| {
        h["sections"][2]["raw"] = 7.into();
    }));
    assert!(matches!(e, ArtifactError::Corrupt(_)), "{e}");
    // A digest that cannot be computed here.
    std::fs::write(&file, &good).unwrap();
    assert!(
        matches!(
            artifact::unpack(
                &file,
                &out,
                &there,
                &|_| Err("no languages".into()),
                &[],
                false
            ),
            Err(ArtifactError::Incompatible(_))
        ),
        "a digest that cannot be computed rejects"
    );
    assert!(turborepo_trace_store::Manifest::read(&out).is_none());

    // After all of that a build is a normal cold build, and the good artifact still
    // works.
    let r = refreshed_equals_cold(&there, "after rejected artifacts");
    assert!(r.traced > 0 && r.analyzed_files > 0);
}

/// Where byte `at` of the artifact `bytes` lies: the fixed head, the header,
/// its checksum, or a chunk of a section (with the offset in the chunk's
/// frame).
fn place(bytes: &[u8], h: &Header, at: usize) -> String {
    let n = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    if at < 8 {
        return format!("magic, byte {at}");
    }
    if at < 12 {
        return format!("header length, byte {}", at - 8);
    }
    if at < 12 + n {
        return format!("header, byte {} of {n}", at - 12);
    }
    if at < 12 + n + 8 {
        return format!("header checksum, byte {}", at - 12 - n);
    }
    let mut pos = 12 + n + 8;
    for s in &h.sections {
        for (i, c) in s.chunks.iter().enumerate() {
            let end = pos + c.zst as usize;
            if at < end {
                return format!(
                    "{}, chunk {i} (a frame of {} bytes), byte {}",
                    s.name,
                    c.zst,
                    at - pos
                );
            }
            pos = end;
        }
    }
    format!("after the last chunk, byte {}", at - pos)
}

/// Whether `file`, tried as an artifact for the checkout `root`, is accepted:
/// `unpack` succeeds, or leaves something of a graph in `out` (which is then
/// removed). `digest` is the options digest of this host for the checkout
/// (computed once: this runs often).
fn installs(file: &Path, root: &Path, out: &Path, digest: &str) -> bool {
    let ok = artifact::unpack(file, out, root, &|_| Ok(digest.to_owned()), &[], false).is_ok();
    let left = turborepo_trace_store::Manifest::read(out).is_some()
        || ["store/pack", "cache/composed", "cache/tables"]
            .iter()
            .chain(artifact::CHECKED)
            .any(|f| out.join(f).exists());
    if ok || left {
        let _ = std::fs::remove_dir_all(out);
    }
    ok || left
}

/// A workflow whose command lines name the checkout's directory: the root
/// package's shard then holds that path ([`artifact::Located`]).
const WORKFLOW_NAMING_THE_ROOT: &str =
    "name: ci\non: push\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: \
     actions/checkout@v4\n      - run: cat ${{ github.workspace }}/docs/guide.md\n      - run: \
     REPORT=\"${{ github.workspace }}/report.json\" node gen.js\n";

/// `unpack` fails closed for every damaged byte: an artifact that differs from
/// a packed one in one byte, whichever byte of the file it is, or that is cut
/// off where a section or a chunk ends (or a byte before or after), installs
/// nothing. Every byte is flipped in one bit (which bit follows the offset, so
/// each of the eight is used throughout), and in each of its bits at the places
/// with a structure of their own: the magic, the header length, and the first
/// bytes of every frame (bit 4 of a zstd frame's fifth byte is unused, and a
/// decoder accepts either value: the content checksum alone let it pass).
///
/// Twice: an artifact with the caches and a recorded run
/// ([`artifact::CHECKED`]), every byte; and one whose root shard names its
/// checkout, restored elsewhere, where the caches are not installed: their
/// bytes, and the recorded run's after them.
#[test]
fn no_damaged_byte_installs_anything() {
    for located in [false, true] {
        let p = Packed::new(|r| {
            fixture(r);
            if located {
                write(r, ".github/workflows/ci.yml", WORKFLOW_NAMING_THE_ROOT);
            }
        });
        // What a query leaves next to the graph, as far as the store is concerned.
        let run: Vec<u8> = (0..600u32)
            .flat_map(|i| (i * i % 251).to_le_bytes())
            .collect();
        let kept = p.root.join(".turbotrace").join(artifact::CHECKED[0]);
        std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
        std::fs::write(&kept, &run).unwrap();
        let path = p.pack("graph.tta", true);
        let bytes = std::fs::read(&path).unwrap();
        let h = artifact::read_header(&path).unwrap();
        assert_eq!(h.located.is_some(), located);
        assert_eq!(h.sections.last().unwrap().name, artifact::CHECKED[0]);
        let there = p.elsewhere("b");
        let out = there.join(".turbotrace");
        let digest = expect(&there, &out)(&h).unwrap();
        let installs = |file: &Path| installs(file, &there, &out, &digest);
        assert!(
            installs(&path),
            "the artifact as it was packed is installed"
        );

        // One byte of the file at a time, written in place.
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        let put = |at: usize, byte: u8| {
            use turborepo_trace_core::os::FileExt;
            file.write_all_at(&[byte], at as u64).unwrap();
        };
        // Where the chunks start, and where each one ends; the first chunk of each
        // section.
        let n = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let mut ends = vec![12 + n + 8];
        let mut sections = Vec::new();
        for s in &h.sections {
            sections.push((s.name.as_str(), *ends.last().unwrap()));
            for c in &s.chunks {
                ends.push(ends.last().unwrap() + c.zst as usize);
            }
        }
        assert_eq!(*ends.last().unwrap(), bytes.len());
        let start_of = |name: &str| sections.iter().find(|s| s.0 == name).map(|s| s.1);
        // Elsewhere, the caches of a located artifact are the sections that are not
        // used.
        let from = match located {
            false => 0,
            true => start_of("cache/composed").expect("packed with the caches"),
        };
        let mut accepted = Vec::new();
        for at in from..bytes.len() {
            let every = at < 12 || ends.iter().any(|e| (*e..e + 8).contains(&at));
            let mut bits = 0u8;
            for bit in 0..8 {
                if !every && bit != at % 8 {
                    continue;
                }
                put(at, bytes[at] ^ (1 << bit));
                if installs(&path) {
                    bits |= 1 << bit;
                }
            }
            put(at, bytes[at]);
            if bits != 0 {
                accepted.push(format!(
                    "file byte {at}, bits {bits:#010b} flipped one at a time: {}",
                    place(&bytes, &h, at)
                ));
            }
        }
        // Cut off.
        let cut = p.scratch.join("cut.tta");
        for end in ends {
            for len in [end - 1, end, end + 1] {
                if len >= bytes.len() {
                    continue;
                }
                std::fs::write(&cut, &bytes[..len]).unwrap();
                if installs(&cut) {
                    accepted.push(format!(
                        "cut off after {len} of {} bytes: before {}",
                        bytes.len(),
                        place(&bytes, &h, len)
                    ));
                }
            }
        }
        assert!(
            accepted.is_empty(),
            "{} damaged artifact(s) of {} bytes were installed (located: {located}; sections: \
             {}):\n{}",
            accepted.len(),
            bytes.len(),
            h.sections
                .iter()
                .map(|s| format!("{} {} chunk(s)", s.name, s.chunks.len()))
                .collect::<Vec<_>>()
                .join(", "),
            accepted.join("\n")
        );
    }
}

#[test]
fn an_existing_graph_is_kept_unless_forced() {
    let p = Packed::new(fixture);
    let file = p.pack("graph.tta", false);
    let there = p.elsewhere("b");
    let out = there.join(".turbotrace");
    write(&there, "packages/b/src/index.ts", "export default 4;\n");
    build(&opts(&there, &out), &langs()).unwrap();
    let before = std::fs::read(out.join("graph.json")).unwrap();
    assert!(matches!(
        unpack(&file, &there, &out),
        Err(ArtifactError::Exists(_))
    ));
    assert_eq!(before, std::fs::read(out.join("graph.json")).unwrap());
    // Forced: the artifact's graph replaces it, and a refresh brings it to the tree
    // again.
    artifact::unpack(&file, &out, &there, &expect(&there, &out), &[], true).unwrap();
    assert_ne!(before, std::fs::read(out.join("graph.json")).unwrap());
    let r = refreshed_equals_cold(&there, "forced over an existing graph");
    assert_eq!(r.traced, 1);
}

#[test]
fn extra_files_travel_by_name_only_when_asked_for() {
    let p = Packed::new(fixture);
    let dir = p.root.join(".turbotrace");
    std::fs::write(dir.join("lang-args.json"), "{\"tags\":[\"x\"]}").unwrap();
    let file = p.scratch.join("graph.tta");
    artifact::pack(
        &dir,
        &file,
        &PackOptions {
            extra: vec!["lang-args.json".into(), "absent.json".into()],
            ..Default::default()
        },
    )
    .unwrap();
    let h = artifact::read_header(&file).unwrap();
    assert_eq!(h.extra.len(), 1);
    assert_eq!(h.label, None);
    let there = p.elsewhere("b");
    let out = there.join(".turbotrace");
    artifact::unpack(
        &file,
        &out,
        &there,
        &expect(&there, &out),
        &["lang-args.json"],
        false,
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(out.join("lang-args.json")).unwrap(),
        "{\"tags\":[\"x\"]}"
    );
    // Not asked for: not written, whatever the header names.
    let other = p.elsewhere("c");
    let out = other.join(".turbotrace");
    unpack(&file, &other, &out).unwrap();
    assert!(!out.join("lang-args.json").exists());
}

#[test]
fn the_digest_computed_without_a_build_is_the_builds() {
    let t = tempfile::tempdir().unwrap();
    let r = canonical(t.path());
    fixture(&r);
    for (i, o) in [
        opts(&r, &r.join("g0")),
        GraphOptions {
            conditions: vec!["development".into()],
            include_types: false,
            ..opts(&r, &r.join("g1"))
        },
        GraphOptions {
            target: Some("linux/arm64".into()),
            externals: Externals::Follow,
            max_glob: 17,
            ..opts(&r, &r.join("g2"))
        },
    ]
    .into_iter()
    .enumerate()
    {
        let built = build(&o, &langs()).unwrap().manifest.options_digest;
        let computed = turborepo_trace_store::build::options_digest_of(&o, &langs()).unwrap();
        assert_eq!(built, computed, "options {i}");
    }
}

/// A shard's bytes are a function of its key, also for what a walk-up did not
/// look at.
///
/// At a level of a `node_modules` walk-up the resolver looks for
/// `<name><suffix>` files only where the `node_modules` directory exists and,
/// for `@scope/name`, the scope directory does. So a `node_modules` or scope
/// directory that appears (another package gets installed or linked there)
/// changes what a trace records, although no path anything probed exists yet. A
/// refresh has to notice, or a shard kept across the change (in a store, or in
/// an artifact packed before it) differs from the one a fresh build writes.
/// Found on a real monorepo: `/// <reference types="@types/x" />` in a package
/// whose `node_modules/@types` appeared with a workspace link.
#[test]
fn a_marker_or_scope_directory_that_appears_retraces_its_walk_ups() {
    let t = tempfile::tempdir().unwrap();
    let base = canonical(t.path());
    let r = base.join("repo");
    write(&r, ".gitignore", "node_modules\n.turbotrace\n");
    write(
        &r,
        "package.json",
        r#"{"name":"root","private":true,"workspaces":["packages/*"]}"#,
    );
    write(&r, "packages/a/package.json", r#"{"name":"a"}"#);
    write(
        &r,
        "packages/a/src/t.ts",
        "/// <reference types=\"@types/absent\" />\nimport s from '@scope/missing';\nexport \
         default s;\n",
    );
    write(&r, "packages/b/package.json", r#"{"name":"b"}"#);
    write(
        &r,
        "packages/b/index.ts",
        "import m from 'missing-pkg';\nexport default m;\n",
    );
    // `packages/a` has a `node_modules` directory (a workspace link); nothing above
    // `packages/b` has one.
    symlink(&r, "../../b", "packages/a/node_modules/b");
    let out = r.join(".turbotrace");
    build(&opts(&r, &out), &langs()).unwrap();
    let file = base.join("before.tta");
    artifact::pack(&out, &file, &PackOptions::default()).unwrap();

    // Other packages of the two scopes are installed in `packages/a`.
    for scope in ["@types", "@scope"] {
        write(
            &r,
            &format!("packages/a/node_modules/{scope}/other/package.json"),
            &format!(r#"{{"name":"{scope}/other","version":"1.0.0"}}"#),
        );
    }
    let report = refreshed_equals_cold(&r, "scope directories appeared");
    assert_eq!(report.traced, 1, "packages/a");
    // A first `node_modules` directory above `packages/b`.
    write(
        &r,
        "packages/b/node_modules/zzz/package.json",
        r#"{"name":"zzz","version":"1.0.0"}"#,
    );
    let report = refreshed_equals_cold(&r, "a node_modules directory appeared");
    assert_eq!(report.traced, 1, "packages/b");
    // The artifact packed before all of it, restored now.
    std::fs::remove_dir_all(&out).unwrap();
    unpack(&file, &r, &out).unwrap();
    let report = refreshed_equals_cold(&r, "restored from before the directories appeared");
    assert_eq!(report.traced, 2);
    // And gone again.
    std::fs::remove_dir_all(r.join("packages/a/node_modules/@types")).unwrap();
    std::fs::remove_dir_all(r.join("packages/b/node_modules")).unwrap();
    refreshed_equals_cold(&r, "the directories are gone again");
}

/// A shard that names the directory of its checkout is not reused in another
/// one.
///
/// A CI workflow's `${{ github.workspace }}` is stored expanded in the task's
/// command lines. The key does not see it (the same tree elsewhere has the same
/// keys), so an artifact would carry the first checkout's path into the second.
/// `pack` lists such shards, and `unpack` at another root makes the store
/// forget them.
#[test]
fn a_shard_that_names_its_checkout_is_traced_again_elsewhere() {
    let p = Packed::new(|r| {
        fixture(r);
        write(r, ".github/workflows/ci.yml", WORKFLOW_NAMING_THE_ROOT);
    });
    let file = p.pack("graph.tta", true);
    let h = artifact::read_header(&file).unwrap();
    let located = h
        .located
        .as_ref()
        .expect("the workflow's shard names the root");
    assert_eq!(located.shards.len(), 1, "the root package's shard");

    // The same location (a fresh graph directory): nothing is left out.
    let same = p.scratch.join("g-same");
    let r = artifact::unpack(&file, &same, &p.root, &expect(&p.root, &same), &[], false).unwrap();
    assert_eq!(r.relocated, 0);
    assert!(same.join("cache/composed").exists());
    let r = build(&opts(&p.root, &same), &langs()).unwrap();
    assert_eq!((r.traced, r.written), (0, 0));

    // Another location: the shard is traced again, with this checkout's path, and
    // the packed caches (composed from the other one) stay behind.
    let there = p.elsewhere("b");
    let out = there.join(".turbotrace");
    let r = unpack(&file, &there, &out).unwrap();
    assert_eq!(r.relocated, 1);
    assert!(!out.join("cache/composed").exists());
    let r = refreshed_equals_cold(&there, "a shard that names its checkout");
    assert_eq!((r.traced, r.written), (1, 1));
    // Packed here, the artifact names this checkout.
    let again = there.join("../again.tta");
    artifact::pack(&out, &again, &PackOptions::default()).unwrap();
    let h2 = artifact::read_header(&again).unwrap();
    let l2 = h2.located.expect("located here too");
    assert_eq!(l2.shards, located.shards, "the same key");
    assert_ne!(l2.root, located.root);
}

/// The jest tests of a graph (repo-relative).
fn jest_tests(out: &Path) -> Vec<String> {
    use turborepo_trace_graph::GraphView;
    let g = Graph::open(out).unwrap();
    g.tests(Some("jest"))
        .into_iter()
        .map(|n| g.path(n).to_owned())
        .collect()
}

/// jest matches its test patterns against the absolute path of a file:
/// `testMatch: ['**/src/**/*.test.js']` takes every `*.test.js` of a checkout
/// below a directory called `src`. At a place the pattern does not name,
/// nothing about the place is part of the graph: the shard holds no root and is
/// reused wherever the place decides nothing either. Restored below a directory
/// called `src`, the shard is traced again (its key covers what the place
/// decides there), the graph is the one a build there makes, and
/// its shard holds that root: packed there and restored at an ordinary place,
/// it is traced again once more.
#[test]
fn a_pattern_that_the_place_decides_is_evaluated_where_the_graph_is_restored() {
    let p = Packed::new(|r| {
        write(r, ".gitignore", "node_modules\n.turbotrace\n");
        write(r, "package.json", r#"{"name":"root","version":"1.0.0"}"#);
        write(
            r,
            "jest.config.js",
            "module.exports = { testMatch: ['**/src/**/*.test.js'] };\n",
        );
        write(r, "lib.js", "module.exports = 1;\n");
        write(
            r,
            "src/a.test.js",
            "require('../lib');\ntest('a', () => {});\n",
        );
        write(
            r,
            "tools/x.test.js",
            "require('../lib');\ntest('x', () => {});\n",
        );
    });
    assert!(!p.root.iter().any(|c| c == "src"), "{}", p.root.display());
    assert_eq!(jest_tests(&p.root.join(".turbotrace")), ["src/a.test.js"]);
    let file = p.pack("graph.tta", true);
    let h = artifact::read_header(&file).unwrap();
    assert!(h.located.is_none(), "an ordinary place is in no shard");

    // Another ordinary place: reused whole.
    let same = p.elsewhere("ordinary");
    let u = unpack(&file, &same, &same.join(".turbotrace")).unwrap();
    assert_eq!(u.relocated, 0);
    let r = refreshed_equals_cold(&same, "a pattern at another ordinary place");
    assert_eq!((r.traced, r.written), (0, 0));

    // Below a directory called `src`: every `*.test.js` is a test there.
    let there = p.scratch.join("named/src/checkout");
    copy_tree(&p.root, &there);
    let out = there.join(".turbotrace");
    unpack(&file, &there, &out).unwrap();
    let r = refreshed_equals_cold(&there, "a pattern that names the place");
    assert_eq!((r.traced, r.written), (1, 1));
    assert_eq!(jest_tests(&out), ["src/a.test.js", "tools/x.test.js"]);

    // Packed there, the shard names its checkout; at an ordinary place it is traced
    // again.
    let again = p.scratch.join("named.tta");
    artifact::pack(&out, &again, &PackOptions::default()).unwrap();
    let h = artifact::read_header(&again).unwrap();
    assert_eq!(
        h.located.as_ref().map(|l| l.shards.len()),
        Some(1),
        "the shard of the place that decides"
    );
    let back = p.elsewhere("back");
    let u = unpack(&again, &back, &back.join(".turbotrace")).unwrap();
    assert_eq!(u.relocated, 1);
    let r = refreshed_equals_cold(&back, "back at an ordinary place");
    assert_eq!((r.traced, r.written), (1, 1));
    assert_eq!(jest_tests(&back.join(".turbotrace")), ["src/a.test.js"]);

    // And between two places that both decide: another root, another key.
    let other = p.scratch.join("other/src/deeper/checkout");
    copy_tree(&p.root, &other);
    let u = unpack(&again, &other, &other.join(".turbotrace")).unwrap();
    assert_eq!(u.relocated, 1);
    let r = refreshed_equals_cold(&other, "another place that decides");
    assert_eq!((r.traced, r.written), (1, 1));
    assert_eq!(
        jest_tests(&other.join(".turbotrace")),
        ["src/a.test.js", "tools/x.test.js"]
    );
}

/// Every task fixture: restored elsewhere it is reused whole and equals a fresh
/// build.
#[test]
fn task_fixtures_restore_like_a_cold_build() {
    let dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../turborepo-trace-testkit/fixtures/tasks");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.file_type().unwrap().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert!(names.len() > 10);
    for name in names {
        let p = Packed::new(|root| {
            copy_tree(&dir.join(&name), root);
            write(root, ".gitignore", "node_modules\n.turbotrace\n");
        });
        let file = p.pack("graph.tta", true);
        let there = p.elsewhere("b");
        let u = unpack(&file, &there, &there.join(".turbotrace")).unwrap();
        let r = refreshed_equals_cold(&there, &name);
        // Every shard is reused, but those that name the first checkout's directory.
        assert_eq!(
            (r.traced, r.written),
            (u.relocated, u.relocated),
            "{name}: shards traced again"
        );
    }
}

// ---- Random edits between the artifact and the working tree ----

#[derive(Clone, Debug)]
enum Edit {
    Write(usize, Vec<usize>),
    Delete(usize),
    BumpLodash(u8),
    AddPackage,
    Tsconfig(bool),
    Uninstall,
    Script(bool),
    /// Another package of a scope is installed in (or removed from)
    /// `packages/a`: the scope directory appears, no path anything looked
    /// for does.
    Scope(bool),
}

const FILES: &[&str] = &[
    "packages/a/src/index.ts",
    "packages/a/src/util.ts",
    "packages/a/src/missing.ts",
    "packages/a/src/dir/index.ts",
    "packages/a/src/locales/de.json",
    "packages/b/src/index.ts",
    "packages/b/src/other.ts",
    "src/root.ts",
    "packages/c/src/c.ts",
    "tools/x.gen.ts",
    "tools/fixtures/h.json",
];

const LINES: &[&str] = &[
    "import { u } from './util';",
    "import './missing';",
    "import _ from 'lodash';",
    "import b from 'b';",
    "import d from './dir';",
    "const l = require('./locales/' + x + '.json');",
    "import o from '../../b/src/other';",
    "import a from 'a';",
    "import p from '@/util';",
    "import type { T } from './types';",
    "const w = require(name);",
    "/// <reference types=\"@types/absent\" />",
    "import s from '@types/missing';",
];

fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        4 => (0..FILES.len(), proptest::collection::vec(0..LINES.len(), 0..4))
            .prop_map(|(f, l)| Edit::Write(f, l)),
        2 => (0..FILES.len()).prop_map(Edit::Delete),
        1 => (0u8..3).prop_map(Edit::BumpLodash),
        1 => Just(Edit::AddPackage),
        1 => any::<bool>().prop_map(Edit::Tsconfig),
        1 => Just(Edit::Uninstall),
        1 => any::<bool>().prop_map(Edit::Script),
        2 => any::<bool>().prop_map(Edit::Scope),
    ]
}

fn apply(r: &Path, e: &Edit) {
    match e {
        Edit::Write(f, lines) => {
            let mut s: String = lines.iter().map(|&l| format!("{}\n", LINES[l])).collect();
            s.push_str("export default 1;\n");
            write(r, FILES[*f], &s);
        }
        Edit::Delete(f) => {
            let _ = std::fs::remove_file(r.join(FILES[*f]));
        }
        Edit::BumpLodash(v) => lodash(r, &format!("4.17.{}", 30 + v), &format!("sha512-{v}")),
        Edit::AddPackage => write(r, "packages/c/package.json", r#"{"name":"c"}"#),
        Edit::Uninstall => {
            let _ = std::fs::remove_file(r.join("packages/a/node_modules/lodash"));
        }
        Edit::Tsconfig(on) => {
            if *on {
                write(
                    r,
                    "packages/a/tsconfig.json",
                    r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
                );
            } else {
                let _ = std::fs::remove_file(r.join("packages/a/tsconfig.json"));
            }
        }
        Edit::Scope(on) => {
            let dir = r.join("packages/a/node_modules/@types");
            if *on {
                write(
                    r,
                    "packages/a/node_modules/@types/other/package.json",
                    r#"{"name":"@types/other","version":"1.0.0"}"#,
                );
            } else {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
        Edit::Script(on) => write(
            r,
            "packages/b/package.json",
            if *on {
                r#"{"name":"b","main":"src/index.ts","scripts":{"build":"tsc -p .","lint":"eslint src"}}"#
            } else {
                r#"{"name":"b","main":"src/index.ts"}"#
            },
        ),
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    /// The artifact of one tree, restored into a checkout that is any number of edits
    /// away (before and after the restore), then refreshed: a fresh build of that tree.
    #[test]
    fn restored_then_refreshed_equals_fresh(
        before in proptest::collection::vec(edit(), 0..5),
        after in proptest::collection::vec(edit(), 0..3),
        caches in any::<bool>(),
    ) {
        let p = Packed::new(fixture);
        let file = p.pack("graph.tta", caches);
        let there = p.elsewhere("b");
        for e in &before {
            apply(&there, e);
        }
        unpack(&file, &there, &there.join(".turbotrace")).unwrap();
        refreshed_equals_cold(&there, &format!("restored after {before:?}"));
        for e in &after {
            apply(&there, e);
            refreshed_equals_cold(&there, &format!("restored after {before:?}, then {e:?}"));
        }
    }
}
