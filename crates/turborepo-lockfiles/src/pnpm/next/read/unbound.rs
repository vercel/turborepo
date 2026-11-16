//! Port of pnpm `next_format/read/unbound.rs`: the peers declared below a
//! snapshot that nothing binds, which v9 `transitivePeerDependencies` lists
//! beside the bound ones.

use std::collections::{BTreeSet, HashSet};

use super::{super::tarjan, Graph, Sid};

pub(in crate::pnpm::data::next) struct UnboundPeers<'a> {
    below: Vec<HashSet<&'a str>>,
}

impl<'a> Graph<'_, 'a> {
    pub fn unbound_peers(&self) -> UnboundPeers<'a> {
        let own: Vec<Vec<&'a str>> = (0..self.keys.len())
            .map(|sid| self.own_unbound_peers(sid))
            .collect();
        let order = tarjan::components(
            self.keys.len(),
            |sid| self.children[sid].iter().map(|&(_, child)| child).collect(),
            |_| true,
        );
        let mut below: Vec<HashSet<&'a str>> = vec![HashSet::new(); self.keys.len()];
        for component in &order.components {
            let mut set: HashSet<&'a str> = HashSet::new();
            for &member in component {
                set.extend(own[member].iter().copied());
                let outside = self.children[member]
                    .iter()
                    .map(|&(_, child)| child)
                    .filter(|child| {
                        order.component_of.get(child) != order.component_of.get(&member)
                    });
                for child in outside {
                    set.extend(below[child].iter().copied());
                }
            }
            for &member in component {
                below[member].clone_from(&set);
            }
        }
        UnboundPeers { below }
    }

    pub fn unbound_below(&self, sid: Sid, unbound: &UnboundPeers<'a>) -> BTreeSet<&'a str> {
        let declared = &self.doc.packages[self.slots[sid].0].peer_ranges;
        unbound.below[sid]
            .iter()
            .copied()
            .filter(|name| !declared.contains_key(name))
            .collect()
    }

    fn own_unbound_peers(&self, sid: Sid) -> Vec<&'a str> {
        let declared = &self.doc.packages[self.slots[sid].0].peer_ranges;
        let entry = self.entry(sid);
        let satisfied = |name: &str| {
            [
                &entry.peer_dependencies,
                &entry.dependencies,
                &entry.optional_dependencies,
            ]
            .into_iter()
            .flatten()
            .any(|map| map.contains_key(name))
        };
        declared
            .keys()
            .copied()
            .filter(|name| !satisfied(name))
            .collect()
    }
}
