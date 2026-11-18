//! Lazily built secondary indexes: probe paths and directory memberships.

use std::collections::HashMap;

use crate::{
    glob::Glob,
    view::{GraphView, NodeId},
};

/// Every way of reading `path` as "something at or below a directory": `(dir,
/// rest)` pairs from the deepest directory up to the root (`""`).
pub(crate) fn dir_splits(path: &str) -> impl Iterator<Item = (&str, &str)> {
    let slashes = path.rmatch_indices('/').map(|(i, _)| i);
    slashes
        .map(move |i| (&path[..i], &path[i + 1..]))
        .chain(std::iter::once(("", path)))
}

/// Sorted `(probe path, node)` pairs. A probe matches an added path when it
/// equals the path or is one of its ancestor directories (a missing directory
/// that the add creates), so a lookup is one binary search per path component.
pub(crate) struct ProbeIndex<'g> {
    entries: Vec<(&'g str, NodeId)>,
}

impl<'g> ProbeIndex<'g> {
    pub(crate) fn build<G: GraphView>(g: &'g G) -> Self {
        let mut entries = Vec::new();
        for i in 0..g.node_count() {
            let n = NodeId(i);
            for p in g.probes(n) {
                entries.push((p, n));
            }
        }
        entries.sort_unstable();
        entries.dedup();
        ProbeIndex { entries }
    }

    /// Calls `f(node, probe)` for every probe matched by adding `path`.
    pub(crate) fn matches(&self, path: &str, mut f: impl FnMut(NodeId, &'g str)) {
        let candidates = std::iter::once(path).chain(dir_splits(path).map(|(d, _)| d));
        for c in candidates {
            if c.is_empty() {
                continue;
            }
            let lo = self.entries.partition_point(|(p, _)| *p < c);
            for &(p, n) in &self.entries[lo..] {
                if p != c {
                    break;
                }
                f(n, p);
            }
        }
    }

    /// Calls `f(node, probe)` for every probe below `path` (`path/…`).
    pub(crate) fn below(&self, path: &str, mut f: impl FnMut(NodeId, &'g str)) {
        let prefix = format!("{path}/");
        let lo = self.entries.partition_point(|(p, _)| *p < prefix.as_str());
        for &(p, n) in &self.entries[lo..] {
            if !p.starts_with(&prefix) {
                break;
            }
            f(n, p);
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Sorted `(input path, node)` pairs ([`GraphView::inputs`]): an exact lookup
/// per changed path.
pub(crate) struct InputIndex<'g> {
    entries: Vec<(&'g str, NodeId)>,
}

impl<'g> InputIndex<'g> {
    pub(crate) fn build<G: GraphView>(g: &'g G) -> Self {
        let mut entries = Vec::new();
        for i in 0..g.node_count() {
            let n = NodeId(i);
            for p in g.inputs(n) {
                entries.push((p, n));
            }
        }
        entries.sort_unstable();
        entries.dedup();
        InputIndex { entries }
    }

    /// Calls `f(node)` for every node with `path` as an input.
    pub(crate) fn matches(&self, path: &str, mut f: impl FnMut(NodeId)) {
        let lo = self.entries.partition_point(|(p, _)| *p < path);
        for &(p, n) in &self.entries[lo..] {
            if p != path {
                break;
            }
            f(n);
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Directory → `(pattern id, node)` memberships. A changed path is tested
/// against the memberships of each of its ancestor directories, with the
/// remainder of the path matched against the pattern. Invalid patterns match
/// everything below their directory (conservative).
pub(crate) struct MembershipIndex<'g> {
    by_dir: HashMap<&'g str, Vec<(u32, NodeId)>>,
    patterns: Vec<(&'g str, Option<Glob>)>,
}

impl<'g> MembershipIndex<'g> {
    pub(crate) fn build<G: GraphView>(g: &'g G) -> Self {
        let mut by_dir: HashMap<&'g str, Vec<(u32, NodeId)>> = HashMap::new();
        let mut pattern_ids: HashMap<&'g str, u32> = HashMap::new();
        let mut patterns = Vec::new();
        for i in 0..g.node_count() {
            let n = NodeId(i);
            for m in g.memberships(n) {
                let id = *pattern_ids.entry(m.pattern).or_insert_with(|| {
                    patterns.push((m.pattern, Glob::new(m.pattern).ok()));
                    patterns.len() as u32 - 1
                });
                by_dir.entry(m.dir).or_default().push((id, n));
            }
        }
        MembershipIndex { by_dir, patterns }
    }

    /// Calls `f(node, dir, pattern)` for every membership matched by adding or
    /// deleting `path`.
    pub(crate) fn matches(&self, path: &str, mut f: impl FnMut(NodeId, &'g str, &'g str)) {
        if self.by_dir.is_empty() {
            return;
        }
        for (dir, rest) in dir_splits(path) {
            let Some((&d, list)) = self.by_dir.get_key_value(dir) else {
                continue;
            };
            for &(pid, n) in list {
                let (src, glob) = &self.patterns[pid as usize];
                if glob.as_ref().is_none_or(|g| g.matches(rest)) {
                    f(n, d, src);
                }
            }
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.by_dir.values().map(Vec::len).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits() {
        let v: Vec<_> = dir_splits("a/b/c.ts").collect();
        assert_eq!(v, [("a/b", "c.ts"), ("a", "b/c.ts"), ("", "a/b/c.ts")]);
        assert_eq!(dir_splits("x").collect::<Vec<_>>(), [("", "x")]);
    }
}
