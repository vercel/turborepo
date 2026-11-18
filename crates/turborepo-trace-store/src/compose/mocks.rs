//! Manual mocks that files other than tests register: an edge from the mocked
//! module to its manual mock ([`Graph::mock_patch`]).
//!
//! `jest.mock('./db')` / `vi.mock('./db')` without a factory loads nothing. The
//! runner loads the manual mock (`__mocks__/db`) when `./db` is first requested
//! after the call (the runtime mock oracle). When a helper or a file a setup
//! file imports makes that call, the tests that run the mock are the ones that
//! request the module, not the ones that load the helper. A test's own call is
//! an edge of the test; a helper's is a fact about another file, the module, so
//! it cannot be stored with the helper's references. The framework layer leaves
//! a marker on the registering file (`mock:manual <runner> <mock> for
//! <module>`, an unresolved edge), and this layer turns every marker into an
//! edge
//!
//! ```text
//! <module> → <mock>      <runner>:manual_mock <mock> registered by <file>
//! ```
//!
//! Whatever reaches the module then reaches the mock: changes, uncertainty and
//! names travel as for an import. `affected` narrows it for tests: the edge
//! counts for a test that also loads a registering file
//! (`turborepo_trace_graph::barrel::mocks::lazy`).
//!
//! The layer is a function of the composed graph: [`Graph::mock_patch_cached`]
//! keeps it in the store's cache of derived layers.

use turborepo_trace_graph::{
    EdgeAttr, GraphView, NodeId, Patch, Phase, RefFlags, RefKind,
    barrel::mocks::{TAG_HAZARD, TAG_INDEX, TAG_MODULE},
};

use super::{Graph, NONE};
use crate::shard::kind;

/// The prefix of every marker's specifier.
const MARK: &str = "mock:";

/// The marker of a manual mock a file registers.
const MANUAL: &str = "mock:manual ";

impl Graph {
    /// The edges from mocked modules to the manual mocks that files other than
    /// tests register for them (see the module documentation), and their
    /// number. Apply it with [`turborepo_trace_graph::Patched`].
    ///
    /// The patch also tags what this pass over every node's markers finds, so
    /// that `affected` does not make the pass again on every call
    /// ([`turborepo_trace_graph::GraphView::tagged`]): the modules that get an
    /// edge, the files with a `mock:hazard` marker, and per module the
    /// files with its `mock:actual` marker.
    pub fn mock_patch(&self) -> (Patch, usize) {
        let mut patch = Patch::new();
        let mut edges = 0;
        let attr = EdgeAttr::new(RefKind::Require, Phase::Runtime, RefFlags::NONE);
        for (ni, node) in self.nodes.iter().enumerate() {
            if node.shard == NONE {
                continue;
            }
            let a = self.shards[node.shard as usize].get();
            if a.package.kind != kind::FIRST_PARTY {
                continue;
            }
            let file = NodeId(ni as u32);
            for e in self.out_edges(file) {
                if e.to.is_some() {
                    continue;
                }
                let Some(spec) = e.specifier.filter(|s| s.starts_with(MARK)) else {
                    continue;
                };
                if spec.starts_with("mock:hazard ") {
                    patch.tag(file, TAG_HAZARD);
                } else if spec.starts_with("mock:actual ") {
                    patch.tag(file, spec);
                }
                let Some((runner, mock, module)) = parse(spec) else {
                    continue;
                };
                let (Some(m), Some(mm)) = (self.node_by_path(module), self.node_by_path(mock))
                else {
                    continue;
                };
                patch.edge(
                    m,
                    mm,
                    attr,
                    &format!(
                        "{runner}:manual_mock {mock} registered by {}",
                        self.path(file)
                    ),
                );
                patch.tag(m, TAG_MODULE);
                edges += 1;
            }
        }
        // The pass was made: an absent tag means that no node has it.
        if !self.nodes.is_empty() {
            patch.tag(NodeId(0), TAG_INDEX);
        }
        (patch, edges)
    }

    /// [`Graph::mock_patch`] through the store's cache of derived layers
    /// ([`Graph::carried_patch`]). The last value: read from the cache.
    ///
    /// The layer reads what composition reads and nothing else: which files are
    /// first party, the specifiers of their unresolved edges (the markers),
    /// paths. No line, no span, no hash, no text of a file. So the layer of
    /// the snapshot before an edit that kept the composed graph is this
    /// graph's whatever files changed, and the pass over every edge is not
    /// made again.
    pub fn mock_patch_cached(&self) -> (Patch, usize, bool) {
        let (patch, meta, kept) = self.carried_patch(
            "mocks",
            3,
            |_, _| true,
            || {
                let (p, n) = self.mock_patch();
                (p, vec![n as u64])
            },
        );
        (
            patch,
            meta.first().copied().unwrap_or(0) as usize,
            kept != super::Kept::Computed,
        )
    }
}

/// The runner, the mock and the module of a marker (`mock:manual <runner>
/// <mock> for <module>`).
fn parse(spec: &str) -> Option<(&str, &str, &str)> {
    let rest = spec.strip_prefix(MANUAL)?;
    let (runner, rest) = rest.split_once(' ')?;
    let (mock, module) = rest.rsplit_once(" for ")?;
    Some((runner, mock, module))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_parse() {
        assert_eq!(
            parse("mock:manual jest src/__mocks__/a b.ts for src/a b.ts"),
            Some(("jest", "src/__mocks__/a b.ts", "src/a b.ts"))
        );
        assert_eq!(parse("mock:actual src/a.ts"), None);
        assert_eq!(parse("mock:manual jest src/a.ts"), None);
    }
}
