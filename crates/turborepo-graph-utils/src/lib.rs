//! Additional utilities to be used with `petgraph`
//! Provides transitive closure calculation and cycle detection with cut
//! candidates to break cycles

mod walker;

use std::{collections::HashSet, fmt::Display, hash::Hash};

use fixedbitset::FixedBitSet;
use itertools::Itertools;
use petgraph::{
    prelude::*,
    visit::{EdgeFiltered, IntoNeighbors, Reversed, VisitMap, Visitable, depth_first_search},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Cyclic dependency detected:\n{cycle_lines}")]
    CyclicDependencies { cycle_lines: String },
    #[error("{0} depends on itself")]
    SelfDependency(String),
}

pub fn transitive_closure<N: Hash + Eq + PartialEq, I: IntoIterator<Item = NodeIndex>>(
    graph: &Graph<N, ()>,
    indices: I,
    direction: petgraph::Direction,
) -> HashSet<&N> {
    let mut visited = HashSet::new();

    let visitor = |event| {
        if let petgraph::visit::DfsEvent::Discover(n, _) = event {
            visited.insert(
                graph
                    .node_weight(n)
                    .expect("node index found during dfs doesn't exist"),
            );
        }
    };

    match direction {
        petgraph::Direction::Outgoing => depth_first_search(&graph, indices, visitor),
        petgraph::Direction::Incoming => depth_first_search(Reversed(&graph), indices, visitor),
    };

    visited
}

pub struct Cycle<N> {
    pub nodes: Vec<NodeIndex>,
    pub cuts: Vec<HashSet<(N, N)>>,
}

/// Given a graph will look for cycles and edge sets that will remove the cycle.
///
/// If any cycles are found they will be returned alongside a list of sets of
/// edges that if they are cut will result in the graph no longer having a
/// cycle.
/// We return (N, N) tuples instead of edge indexes as indexes are not stable
/// across removals so the indexes in the subgraph do not match the full graph.
pub fn cycles_and_cut_candidates<N: Clone + Hash + Eq, E: Clone>(
    graph: &Graph<N, E>,
) -> Vec<Cycle<N>> {
    petgraph::algo::tarjan_scc(graph)
        .into_iter()
        .filter(|cycle| cycle.len() > 1)
        .map(|nodes| {
            let mut subgraph = graph.clone();
            subgraph.retain_nodes(|_, node| nodes.contains(&node));
            let cuts = edges_to_break_cycle(&subgraph);
            Cycle { nodes, cuts }
        })
        .collect()
}

/// Given a graph that has a cycle, return all minimal sets of edges that result
/// in a cycle no longer being present.
/// Minimal here means that if the cycle can be broken by only removing n edges,
/// then only sets containing n edges will be returned.
fn edges_to_break_cycle<N: Clone + Hash + Eq, E: Clone>(
    graph: &Graph<N, E>,
) -> Vec<HashSet<(N, N)>> {
    let edge_sets = graph.edge_indices().powerset();
    let mut breaking_edge_sets = Vec::new();

    // For each DFS
    let mut cycle_detector = CycleDetector::new(graph);

    let mut minimal_break_point = usize::MAX;
    for edge_set in edge_sets {
        let set_size = edge_set.len();
        if set_size > minimal_break_point {
            break;
        }
        let trimmed_graph = EdgeFiltered::from_fn(graph, |edge| !edge_set.contains(&edge.id()));

        let is_cyclic = cycle_detector.has_cycle(&trimmed_graph, trimmed_graph.0.node_indices());
        if !is_cyclic {
            minimal_break_point = set_size;
            breaking_edge_sets.push(
                edge_set
                    .into_iter()
                    .map(|edge| {
                        let (src, dst) = graph.edge_endpoints(edge).unwrap();
                        (
                            graph.node_weight(src).unwrap().clone(),
                            graph.node_weight(dst).unwrap().clone(),
                        )
                    })
                    .collect(),
            );
        }
    }

    breaking_edge_sets
}

pub fn validate_graph<N: Display + Clone + Hash + Eq>(graph: &Graph<N, ()>) -> Result<(), Error> {
    let cycles = cycles_and_cut_candidates(graph);

    let cycle_lines = cycles
        .into_iter()
        .map(|Cycle { nodes, cuts }| {
            let workspaces = nodes.into_iter().map(|id| graph.node_weight(id).unwrap());
            let cuts = cuts.into_iter().map(format_cut).format("\n\t");
            format!(
                "\t{}\n\nThe cycle can be broken by removing any of these sets of \
                 dependencies:\n\t{cuts}",
                workspaces.format(", ")
            )
        })
        .join("\n");

    if !cycle_lines.is_empty() {
        return Err(Error::CyclicDependencies { cycle_lines });
    }

    for edge in graph.edge_references() {
        if edge.source() == edge.target() {
            let node = graph
                .node_weight(edge.source())
                .expect("edge pointed to missing node");
            return Err(Error::SelfDependency(node.to_string()));
        }
    }

    Ok(())
}

fn format_cut<N: Display>(edges: impl IntoIterator<Item = (N, N)>) -> String {
    let edges = edges
        .into_iter()
        .map(|(src, dst)| format!("{src} -> {dst}"))
        .sorted()
        .format(", ");

    format!("{{ {edges} }}")
}

struct CycleDetector {
    visited: FixedBitSet,
    finished: FixedBitSet,
}

impl CycleDetector {
    fn new<N, E>(graph: &Graph<N, E>) -> CycleDetector {
        let visited = graph.visit_map();
        let finished = graph.visit_map();
        Self { visited, finished }
    }

    // A fast failing DFS approach to detecting if there is a cycle left in the
    // graph
    // Used over `petgraph::visit::depth_first_search` as it allows us to reuse
    // visit maps.
    fn has_cycle<G, I>(&mut self, graph: G, starts: I) -> bool
    where
        G: IntoNeighbors + Visitable<Map = FixedBitSet>,
        I: IntoIterator<Item = G::NodeId>,
    {
        self.visited.clear();
        self.finished.clear();
        for start in starts {
            if Self::dfs(graph, start, &mut self.visited, &mut self.finished) {
                return true;
            }
        }
        false
    }

    fn dfs<G>(graph: G, u: G::NodeId, visited: &mut G::Map, finished: &mut G::Map) -> bool
    where
        G: IntoNeighbors + Visitable,
    {
        // We have already completed a DFS from this node
        if finished.is_visited(&u) {
            return false;
        }
        // If not the first visit we have a cycle
        if !visited.visit(u) {
            return true;
        }
        for v in graph.neighbors(u) {
            if Self::dfs(graph, v, visited, finished) {
                return true;
            }
        }
        finished.visit(u);
        false
    }
}

pub use walker::{WalkMessage, Walker};

#[cfg(test)]
mod test {
    use insta::assert_snapshot;
    use petgraph::graph::Graph;

    use super::*;

    #[test]
    fn test_cycle_err_message() {
        /*
         a -> b --> c -> d
         |    |\____/    |
         |     \_______/ |
          \_____________/
        */
        let mut g = Graph::new();
        let a = g.add_node("a");
        let b = g.add_node("b");
        let c = g.add_node("c");
        let d = g.add_node("d");

        g.add_edge(a, b, ());
        g.add_edge(b, c, ());
        g.add_edge(c, b, ());
        g.add_edge(c, d, ());
        g.add_edge(d, b, ());
        g.add_edge(d, a, ());

        let result = validate_graph(&g);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_snapshot!(err.to_string(), @r###"
        Cyclic dependency detected:
        	d, c, b, a

        The cycle can be broken by removing any of these sets of dependencies:
        	{ b -> c }
        "###);
    }

    #[test]
    fn test_basic_cycle_break() {
        // Simple cycle where any edge would break the cycle
        let mut g = Graph::new();
        let a = g.add_node("a");
        let b = g.add_node("b");
        let c = g.add_node("c");

        g.add_edge(a, b, ());
        g.add_edge(b, c, ());
        g.add_edge(c, a, ());

        let breaks = edges_to_break_cycle(&g);
        assert_eq!(breaks.len(), 3, "{breaks:?}");
        let mut edges_that_break = HashSet::new();
        for brk in breaks {
            assert_eq!(brk.len(), 1);
            edges_that_break.extend(brk.into_iter());
        }
        assert_eq!(
            edges_that_break,
            [("a", "b"), ("b", "c"), ("c", "a")]
                .iter()
                .copied()
                .collect::<HashSet<_>>()
        );
    }

    #[test]
    fn test_double_cycle_break() {
        // 2 cycles where only one edge would break the cycle
        let mut g = Graph::new();
        let a = g.add_node("a");
        let b = g.add_node("b");
        let c = g.add_node("c");
        let d = g.add_node("d");

        // Cycle 1
        g.add_edge(a, b, ());
        g.add_edge(b, c, ());
        g.add_edge(c, a, ());

        // Cycle 2
        g.add_edge(b, d, ());
        g.add_edge(d, a, ());

        let breaks = edges_to_break_cycle(&g);
        assert_eq!(breaks.len(), 1, "{breaks:?}");
        assert_eq!(
            breaks.into_iter().flatten().exactly_one().unwrap(),
            ("a", "b")
        );
    }

    #[test]
    fn test_cycle_break_two_edges() {
        // cycle where multiple edges required to break the cycle
        // a,b,c form a cycle with
        // a <-> c and b <-> c
        let mut g = Graph::new();
        let a = g.add_node("a");
        let b = g.add_node("b");
        let c = g.add_node("c");

        g.add_edge(a, b, ());
        g.add_edge(b, c, ());
        g.add_edge(c, a, ());
        g.add_edge(a, c, ());
        g.add_edge(c, b, ());

        let breaks = edges_to_break_cycle(&g);
        assert_eq!(breaks.len(), 3, "{breaks:?}");
        let expected_1: HashSet<_> = [("b", "c"), ("a", "c")].iter().copied().collect();
        let expected_2: HashSet<_> = [("b", "c"), ("c", "a")].iter().copied().collect();
        let expected_3: HashSet<_> = [("c", "b"), ("c", "a")].iter().copied().collect();
        assert!(breaks.contains(&expected_1), "{breaks:?}");
        assert!(breaks.contains(&expected_2), "{breaks:?}");
        assert!(breaks.contains(&expected_3), "{breaks:?}");
    }
}
