//! The composed graph of a manifest, reused across runs (`<store
//! dir>/cache/composed`).
//!
//! Composition is a pure function of the binary ([`crate::build::build_id`])
//! and the manifest's shards (keys, kinds, names, versions and roots, in order)
//! and walked-files list: when those are unchanged (an untouched repository),
//! the nodes, edges, hub expansions and task links are read back instead of
//! recomputed. Shards are still mapped: edge details, uncertainties and tasks
//! are read from them in place.

use turborepo_trace_graph::{Adjacency, EdgeAttr};

use super::{GenOutput, Graph, Node};

const MAGIC: &[u8; 8] = b"DTCOMP1\n";

/// What [`Composed`] is a function of.
pub(super) fn key(m: &crate::manifest::Manifest) -> u128 {
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    h.update(MAGIC);
    h.update(crate::build::build_id().as_bytes());
    h.update(&[0]);
    h.update(m.files.as_deref().unwrap_or("").as_bytes());
    h.update(&[0]);
    h.update(&(m.shards.len() as u64).to_le_bytes());
    for e in &m.shards {
        for s in [&e.key, &e.kind, &e.name, &e.root] {
            h.update(s.as_bytes());
            h.update(&[0]);
        }
        h.update(e.version.as_deref().unwrap_or("\u{1}").as_bytes());
        h.update(&[0]);
    }
    h.digest128()
}

/// The composed parts of a [`Graph`].
pub(super) struct Composed {
    pub paths: String,
    pub path_end: Vec<u32>,
    pub nodes: Vec<Node>,
    pub langs: Vec<String>,
    pub edge_off: Vec<u32>,
    pub edge_to: Vec<u32>,
    pub fwd: Adjacency,
    pub extra_off: Vec<u32>,
    pub extra_to: Vec<u32>,
    pub hub_nodes: Vec<(u32, u32)>,
    pub hub_targets: Vec<Vec<u32>>,
    pub generated: Vec<(u32, Vec<u32>)>,
    pub gen_outputs: Vec<GenOutput>,
}

struct Enc(Vec<u8>);

impl Enc {
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32s(&mut self, v: &[u32]) {
        self.u64(v.len() as u64);
        self.0.reserve(v.len() * 4);
        for x in v {
            self.0.extend_from_slice(&x.to_le_bytes());
        }
    }
    fn str(&mut self, s: &str) {
        self.u64(s.len() as u64);
        self.0.extend_from_slice(s.as_bytes());
    }
    fn opt(&mut self, s: Option<&str>) {
        match s {
            None => self.u64(u64::MAX),
            Some(s) => self.str(s),
        }
    }
}

struct Dec<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Dec<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(s)
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn len(&mut self) -> Option<usize> {
        let n = self.u64()? as usize;
        (n <= self.b.len()).then_some(n)
    }
    fn u32s(&mut self) -> Option<Vec<u32>> {
        let n = self.len()?;
        let b = self.take(n.checked_mul(4)?)?;
        Some(
            b.as_chunks::<4>()
                .0
                .iter()
                .map(|c| u32::from_le_bytes(*c))
                .collect(),
        )
    }
    fn str(&mut self) -> Option<String> {
        let n = self.len()?;
        Some(std::str::from_utf8(self.take(n)?).ok()?.to_owned())
    }
    fn opt(&mut self) -> Option<Option<String>> {
        let n = self.u64()?;
        if n == u64::MAX {
            return Some(None);
        }
        self.pos -= 8;
        self.str().map(Some)
    }
}

fn pairs(v: Vec<u32>) -> Option<Vec<(u32, u32)>> {
    v.len()
        .is_multiple_of(2)
        .then(|| v.as_chunks::<2>().0.iter().map(|c| (c[0], c[1])).collect())
}

impl Composed {
    /// Encodes the composed parts of `g` under `key`.
    pub fn encode(g: &Graph, key: u128) -> Vec<u8> {
        let mut e = Enc(Vec::with_capacity(
            64 + g.paths.len()
                + 4 * (g.path_end.len() * 6 + g.edge_to.len() * 2 + g.fwd.edge_count() * 2),
        ));
        e.0.extend_from_slice(MAGIC);
        e.0.extend_from_slice(&key.to_le_bytes());
        e.str(&g.paths);
        e.u32s(&g.path_end);
        let mut nodes = Vec::with_capacity(g.nodes.len() * 4);
        for n in &g.nodes {
            nodes.extend_from_slice(&[n.pkg, n.shard, n.file, n.lang as u32]);
        }
        e.u32s(&nodes);
        e.u64(g.langs.len() as u64);
        for l in &g.langs {
            e.str(l);
        }
        e.u32s(&g.edge_off);
        e.u32s(&g.edge_to);
        let (off, to, attrs) = g.fwd.parts();
        e.u32s(off);
        e.u32s(to);
        e.u32s(&attrs.iter().map(|a| a.0 as u32).collect::<Vec<_>>());
        e.u32s(&g.extra_off);
        e.u32s(&g.extra_to);
        e.u32s(
            &g.hub_nodes
                .iter()
                .flat_map(|&(a, b)| [a, b])
                .collect::<Vec<_>>(),
        );
        e.u64(g.hub_targets.len() as u64);
        for t in &g.hub_targets {
            e.u32s(t);
        }
        e.u64(g.generated.len() as u64);
        for (n, outs) in &g.generated {
            e.u64(*n as u64);
            e.u32s(outs);
        }
        e.u64(g.gen_outputs.len() as u64);
        for (p, pattern, exclude, package) in &g.gen_outputs {
            e.str(p);
            e.opt(pattern.as_deref());
            e.u64(exclude.len() as u64);
            for x in exclude {
                e.str(x);
            }
            e.opt(package.as_deref());
        }
        let sum = xxhash_rust::xxh3::xxh3_64(&e.0);
        e.0.extend_from_slice(&sum.to_le_bytes());
        e.0
    }

    /// Decodes parts encoded under `key`; `None` for another key, a corrupt
    /// file, or parts that do not fit `shard_files` (files per shard of the
    /// manifest).
    pub fn decode(b: &[u8], key: u128, shard_files: &[usize]) -> Option<Composed> {
        if b.len() < 32 || &b[..8] != MAGIC || b[8..24] != key.to_le_bytes() {
            return None;
        }
        let (body, sum) = b.split_at(b.len() - 8);
        if xxhash_rust::xxh3::xxh3_64(body).to_le_bytes() != sum {
            return None;
        }
        let mut d = Dec { b: body, pos: 24 };
        let paths = d.str()?;
        let path_end = d.u32s()?;
        let raw = d.u32s()?;
        if raw.len() != path_end.len() * 4 {
            return None;
        }
        let nodes: Vec<Node> = raw
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| Node {
                pkg: c[0],
                shard: c[1],
                file: c[2],
                lang: c[3] as u8,
            })
            .collect();
        let nl = d.len()?;
        let mut langs = Vec::with_capacity(nl.min(256));
        for _ in 0..nl {
            langs.push(d.str()?);
        }
        let edge_off = d.u32s()?;
        let edge_to = d.u32s()?;
        let fwd_off = d.u32s()?;
        let fwd_to = d.u32s()?;
        let fwd_attr: Vec<EdgeAttr> = d.u32s()?.into_iter().map(|a| EdgeAttr(a as u16)).collect();
        let extra_off = d.u32s()?;
        let extra_to = d.u32s()?;
        let hub_nodes = pairs(d.u32s()?)?;
        let nh = d.len()?;
        let mut hub_targets = Vec::with_capacity(nh.min(1 << 20));
        for _ in 0..nh {
            hub_targets.push(d.u32s()?);
        }
        let ng = d.len()?;
        let mut generated = Vec::with_capacity(ng.min(1 << 20));
        for _ in 0..ng {
            let n = d.u64()? as u32;
            generated.push((n, d.u32s()?));
        }
        let no = d.len()?;
        let mut gen_outputs = Vec::with_capacity(no.min(1 << 20));
        for _ in 0..no {
            let p = d.str()?;
            let pattern = d.opt()?;
            let ne = d.len()?;
            let mut exclude = Vec::with_capacity(ne.min(1024));
            for _ in 0..ne {
                exclude.push(d.str()?);
            }
            gen_outputs.push((p, pattern, exclude, d.opt()?));
        }
        if d.pos != body.len() {
            return None;
        }
        // Consistency with the shards and within the parts (indexes are used
        // unchecked).
        let n = nodes.len();
        let none = super::NONE;
        let ok = path_end.windows(2).all(|w| w[0] <= w[1])
            && path_end.last().is_none_or(|&e| e as usize == paths.len())
            && path_end.iter().all(|&e| paths.is_char_boundary(e as usize))
            && nodes.iter().all(|x| {
                (x.pkg as usize) < shard_files.len()
                    && (x.shard == none
                        || ((x.shard as usize) < shard_files.len()
                            && (x.file as usize) < shard_files[x.shard as usize]))
            })
            && csr_ok(&edge_off, edge_to.len(), n)
            && edge_to.iter().all(|&t| t == none || (t as usize) < n)
            && csr_ok(&fwd_off, fwd_to.len(), n)
            && fwd_to.len() == fwd_attr.len()
            && fwd_to.iter().all(|&t| (t as usize) < n)
            && csr_ok(&extra_off, extra_to.len(), n)
            && extra_to
                .iter()
                .all(|&t| ((t & !super::TYPE_ONLY_LINK) as usize) < n)
            && hub_nodes
                .iter()
                .all(|&(a, h)| (a as usize) < n && (h as usize) < hub_targets.len())
            && hub_targets.iter().flatten().all(|&t| (t as usize) < n)
            && generated.iter().all(|(a, outs)| {
                (*a as usize) < n && outs.iter().all(|&o| (o as usize) < gen_outputs.len())
            });
        if !ok {
            return None;
        }
        Some(Composed {
            paths,
            path_end,
            nodes,
            langs,
            edge_off,
            edge_to,
            fwd: Adjacency::from_parts(fwd_off, fwd_to, fwd_attr),
            extra_off,
            extra_to,
            hub_nodes,
            hub_targets,
            generated,
            gen_outputs,
        })
    }
}

/// Whether `off` is a CSR offset array over `n` nodes and `len` items.
fn csr_ok(off: &[u32], len: usize, n: usize) -> bool {
    // Empty: not built (no extra edges).
    off.is_empty() && len == 0
        || (off.len() == n + 1
            && off[0] == 0
            && off[n] as usize == len
            && off.windows(2).all(|w| w[0] <= w[1]))
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Arc};

    use turborepo_trace_core::Language;

    use crate::{Graph, GraphOptions, build};

    fn write(root: &Path, rel: &str, contents: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, contents).unwrap();
    }

    /// Generated-file links (a root task's outputs) and membership hubs shared
    /// by tasks.
    fn fixture(r: &Path) {
        write(r, ".gitignore", "node_modules\n.turbotrace\n");
        write(
            r,
            "package.json",
            r#"{"name":"root","private":true,"scripts":{"gen":"node gen.js"}}"#,
        );
        write(r, "gen.js", "console.log(1);\n");
        write(r, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
        write(
            r,
            "turbo.json",
            r#"{"tasks":{"//#gen":{"outputs":["tools/**/*.gen.ts"]}}}"#,
        );
        write(
            r,
            "tools/b.js",
            "const fs = require('fs');\nconst path = require('path');\nmodule.exports = \
             fs.readdirSync(path.join(__dirname, 'fixtures'));\nrequire('./x.gen');\n",
        );
        write(r, "tools/fixtures/g.json", "{}\n");
        for p in ["a", "b"] {
            write(
                r,
                &format!("packages/{p}/package.json"),
                &format!(
                    r#"{{"name":"{p}","version":"1.0.0","scripts":{{"clean-code":"knip --directory ../.. --workspace packages/{p}"}}}}"#
                ),
            );
            write(
                r,
                &format!("packages/{p}/src/index.ts"),
                "import { x } from 'a';\nexport const y = x;\n",
            );
        }
        write(r, "docs/guide.md", "# guide\n");
    }

    #[test]
    fn a_cached_composition_equals_a_fresh_one() {
        let t = tempfile::tempdir().unwrap();
        let r = std::fs::canonicalize(t.path()).unwrap();
        fixture(&r);
        let out = r.join(".turbotrace");
        let opts = GraphOptions {
            root: r.clone(),
            out: Some(out.clone()),
            threads: 2,
            ..Default::default()
        };
        let langs: Vec<Arc<dyn Language>> = vec![Arc::new(turborepo_trace_lang_js::JsLanguage)];
        build(&opts, &langs).unwrap();
        let fresh = Graph::open(&out).unwrap();
        assert!(out.join("cache/composed").exists());
        assert!(!fresh.hub_nodes.is_empty(), "the fixture has hubs");
        assert!(!fresh.extra_to.is_empty(), "the fixture has task links");
        let cached = Graph::open(&out).unwrap();
        assert_eq!(
            cached.stats.link_us, 0,
            "composed parts come from the cache"
        );
        let key = super::key(&fresh.manifest);
        assert_eq!(
            super::Composed::encode(&fresh, key),
            super::Composed::encode(&cached, key)
        );
        assert_eq!(fresh.packages.len(), cached.packages.len());
        // A changed snapshot does not use the stale parts.
        write(&r, "packages/b/src/more.ts", "import './index';\n");
        build(&opts, &langs).unwrap();
        let next = Graph::open(&out).unwrap();
        assert_ne!(next.stats.link_us, 0);
        assert!(
            turborepo_trace_graph::GraphView::node_by_path(&next, "packages/b/src/more.ts")
                .is_some()
        );
        // A corrupt cache is ignored.
        let p = out.join("cache/composed");
        let mut b = std::fs::read(&p).unwrap();
        let mid = b.len() / 2;
        b[mid] ^= 0xff;
        std::fs::write(&p, b).unwrap();
        let again = Graph::open(&out).unwrap();
        assert_ne!(again.stats.link_us, 0);
        let k = super::key(&next.manifest);
        assert_eq!(
            super::Composed::encode(&next, k),
            super::Composed::encode(&again, k)
        );
    }
}
