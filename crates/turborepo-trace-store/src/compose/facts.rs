//! Per-file facts of the composed graph
//! (`turborepo_trace_core::RefSink::fact`): tagged strings a language plugin
//! recorded with a file's extraction, read in place from the shards.

use rayon::prelude::*;
use turborepo_trace_graph::NodeId;

use super::{Graph, NONE};
use crate::shard::kind;

impl Graph {
    /// The facts of the file `n` (empty for nodes that are not traced files).
    pub fn facts(&self, n: NodeId) -> impl Iterator<Item = &str> + '_ {
        self.file(n)
            .and_then(|f| f.extraction.as_ref())
            .into_iter()
            .flat_map(|ex| ex.facts.iter().map(|f| f.as_str()))
    }

    /// Maps every fact of a first-party file that starts with `tag` with `f`,
    /// in parallel; the results that are `Some`, in node order.
    pub fn map_facts<T: Send>(
        &self,
        tag: &str,
        f: &(dyn Fn(NodeId, &str) -> Option<T> + Sync),
    ) -> Vec<T> {
        let chunk = (self.nodes.len() / (rayon::current_num_threads() * 4)).max(1024);
        let parts: Vec<Vec<T>> = (0..self.nodes.len())
            .into_par_iter()
            .step_by(chunk)
            .map(|start| {
                let mut out = Vec::new();
                for ni in start..(start + chunk).min(self.nodes.len()) {
                    let node = self.nodes[ni];
                    if node.shard == NONE {
                        continue;
                    }
                    let a = self.shards[node.shard as usize].get();
                    if a.package.kind != kind::FIRST_PARTY {
                        continue;
                    }
                    let Some(ex) = a
                        .files
                        .get(node.file as usize)
                        .and_then(|file| file.extraction.as_ref())
                    else {
                        continue;
                    };
                    for fact in ex.facts.iter() {
                        let fact = fact.as_str();
                        if fact.starts_with(tag)
                            && let Some(t) = f(NodeId(ni as u32), fact)
                        {
                            out.push(t);
                        }
                    }
                }
                out
            })
            .collect();
        parts.into_iter().flatten().collect()
    }

    /// Calls `f` with every fact of a first-party file that starts with `tag`,
    /// in node order.
    pub fn for_each_fact(&self, tag: &str, f: &mut dyn FnMut(NodeId, &str)) {
        for (ni, node) in self.nodes.iter().enumerate() {
            if node.shard == NONE {
                continue;
            }
            let a = self.shards[node.shard as usize].get();
            if a.package.kind != kind::FIRST_PARTY {
                continue;
            }
            let Some(file) = a.files.get(node.file as usize) else {
                continue;
            };
            let Some(ex) = file.extraction.as_ref() else {
                continue;
            };
            for fact in ex.facts.iter() {
                let fact = fact.as_str();
                if fact.starts_with(tag) {
                    f(NodeId(ni as u32), fact);
                }
            }
        }
    }
}
