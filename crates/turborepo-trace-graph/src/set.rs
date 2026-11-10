//! Dense node sets as bitsets.

use crate::view::NodeId;

/// A set of nodes of one graph, stored as a bitset over `0..universe`.
///
/// Set operations are word-parallel; a 200k-node set is 3125 words.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct NodeSet {
    words: Vec<u64>,
    universe: u32,
}

impl std::fmt::Debug for NodeSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.iter().map(|n| n.0)).finish()
    }
}

impl NodeSet {
    /// An empty set over `universe` nodes.
    pub fn new(universe: u32) -> Self {
        NodeSet {
            words: vec![0; (universe as usize).div_ceil(64)],
            universe,
        }
    }

    /// The set of all nodes.
    pub fn full(universe: u32) -> Self {
        let mut s = NodeSet {
            words: vec![u64::MAX; (universe as usize).div_ceil(64)],
            universe,
        };
        s.trim();
        s
    }

    /// Builds a set from ids.
    pub fn from_iter_in(universe: u32, ids: impl IntoIterator<Item = NodeId>) -> Self {
        let mut s = NodeSet::new(universe);
        for id in ids {
            s.insert(id);
        }
        s
    }

    fn trim(&mut self) {
        let rem = self.universe % 64;
        if rem != 0
            && let Some(last) = self.words.last_mut()
        {
            *last &= (1u64 << rem) - 1;
        }
    }

    /// The universe size (node count of the graph).
    pub fn universe(&self) -> u32 {
        self.universe
    }

    /// Adds `n`; returns whether it was newly inserted.
    #[inline]
    pub fn insert(&mut self, n: NodeId) -> bool {
        let (w, b) = (n.0 as usize / 64, n.0 % 64);
        let word = &mut self.words[w];
        let was = *word & (1 << b) != 0;
        *word |= 1 << b;
        !was
    }

    /// Removes `n`.
    #[inline]
    pub fn remove(&mut self, n: NodeId) {
        self.words[n.0 as usize / 64] &= !(1 << (n.0 % 64));
    }

    /// Whether `n` is in the set.
    #[inline]
    pub fn contains(&self, n: NodeId) -> bool {
        self.words
            .get(n.0 as usize / 64)
            .is_some_and(|w| w & (1 << (n.0 % 64)) != 0)
    }

    /// Removes every element.
    pub fn clear(&mut self) {
        self.words.fill(0);
    }

    /// Number of elements.
    pub fn len(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|&w| w == 0)
    }

    /// `self ∪= other`.
    pub fn union_with(&mut self, other: &NodeSet) {
        debug_assert_eq!(self.universe, other.universe);
        for (a, b) in self.words.iter_mut().zip(&other.words) {
            *a |= b;
        }
    }

    /// `self ∩= other`.
    pub fn intersect_with(&mut self, other: &NodeSet) {
        debug_assert_eq!(self.universe, other.universe);
        for (a, b) in self.words.iter_mut().zip(&other.words) {
            *a &= b;
        }
    }

    /// `self -= other`.
    pub fn difference_with(&mut self, other: &NodeSet) {
        debug_assert_eq!(self.universe, other.universe);
        for (a, b) in self.words.iter_mut().zip(&other.words) {
            *a &= !b;
        }
    }

    /// Whether the two sets share an element.
    pub fn intersects(&self, other: &NodeSet) -> bool {
        self.words.iter().zip(&other.words).any(|(a, b)| a & b != 0)
    }

    /// Keeps only elements for which `f` returns true.
    pub fn retain(&mut self, mut f: impl FnMut(NodeId) -> bool) {
        for (wi, word) in self.words.iter_mut().enumerate() {
            let mut w = *word;
            while w != 0 {
                let b = w.trailing_zeros();
                w &= w - 1;
                if !f(NodeId(wi as u32 * 64 + b)) {
                    *word &= !(1 << b);
                }
            }
        }
    }

    /// Elements in increasing id order.
    pub fn iter(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.words.iter().enumerate().flat_map(|(wi, &w)| {
            let mut w = w;
            std::iter::from_fn(move || {
                if w == 0 {
                    return None;
                }
                let b = w.trailing_zeros();
                w &= w - 1;
                Some(NodeId(wi as u32 * 64 + b))
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops() {
        let mut a = NodeSet::from_iter_in(130, [0, 5, 64, 129].map(NodeId));
        let b = NodeSet::from_iter_in(130, [5, 64, 100].map(NodeId));
        assert_eq!(a.len(), 4);
        assert!(a.intersects(&b));
        let mut i = a.clone();
        i.intersect_with(&b);
        assert_eq!(i.iter().map(|n| n.0).collect::<Vec<_>>(), [5, 64]);
        a.difference_with(&b);
        assert_eq!(a.iter().map(|n| n.0).collect::<Vec<_>>(), [0, 129]);
        a.union_with(&b);
        assert_eq!(a.len(), 5);
        a.retain(|n| n.0 % 2 == 0);
        assert_eq!(a.iter().map(|n| n.0).collect::<Vec<_>>(), [0, 64, 100]);
        assert_eq!(NodeSet::full(130).len(), 130);
        assert!(!a.contains(NodeId(1000)));
    }
}
