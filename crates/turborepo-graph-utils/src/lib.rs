mod walker;

use std::{collections::HashSet, fmt::Display, hash::Hash};

use itertools::Itertools;
use petgraph::{
    prelude::*,
    visit::{depth_first_search, Reversed},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Cyclic dependency detected:\n{cycle_lines}\nDot file saved to {dot:?}")]
    CyclicDependencies {
        cycle_lines: String,
        dot: Option<String>,
    },
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

pub fn some_smart_name<N: Clone, E: Clone>(graph: &Graph<N, E>) -> Option<Vec<HashSet<EdgeIndex>>> {
    let sccs = petgraph::algo::tarjan_scc(graph)
        .into_iter()
        .filter(|cycle| cycle.len() > 1)
        .collect::<Vec<_>>();
    if sccs.is_empty() {
        return None;
    }
    let subgraphs = sccs
        .iter()
        .map(|scc| {
            let mut subgraph = graph.clone();
            subgraph.retain_nodes(|_, node| scc.contains(&node));
            subgraph
        })
        .collect::<Vec<_>>();

    None
}

/// Given a graph that has a cycle, return all minimal sets of edges that result
/// in a cycle no longer being present.
/// Minimal here means that if the cycle can be broken by only removing n edges,
/// then only sets containing n edges will be returned.
fn edges_to_break_cycle<N: Clone, E: Clone>(graph: &Graph<N, E>) -> Vec<HashSet<EdgeIndex>> {
    let edges = graph.edge_indices().collect::<HashSet<_>>();
    let edge_sets = edges.iter().copied().powerset();
    let mut breaking_edge_sets = Vec::new();

    let mut minimal_break_point = usize::MAX;
    for edge_set in edge_sets {
        let set_size = edge_set.len();
        if set_size > minimal_break_point {
            break;
        }
        let mut trimmed_graph = graph.clone();
        for edge in &edge_set {
            trimmed_graph.remove_edge(*edge);
        }

        let is_cyclic = petgraph::algo::tarjan_scc(&trimmed_graph)
            .into_iter()
            .filter(|scc| scc.len() > 1)
            .count()
            > 0;
        if !is_cyclic {
            minimal_break_point = set_size;
            breaking_edge_sets.push(edge_set.into_iter().collect());
        }
    }

    breaking_edge_sets
}

pub fn validate_graph<G: Display + Clone + std::fmt::Debug>(
    graph: &Graph<G, ()>,
) -> Result<(), Error> {
    // This is equivalent to AcyclicGraph.Cycles from Go's dag library
    let sccs = petgraph::algo::tarjan_scc(graph);
    let all_nodes_in_cycle = sccs
        .iter()
        .filter(|cycle| cycle.len() > 1)
        .flatten()
        .copied()
        .collect::<HashSet<_>>();
    let dot = (!all_nodes_in_cycle.is_empty())
        .then(|| {
            let mut subgraph = graph.clone();
            subgraph.retain_nodes(|_, node| all_nodes_in_cycle.contains(&node));
            let dot = format!(
                "{:?}",
                petgraph::dot::Dot::with_config(&subgraph, &[petgraph::dot::Config::EdgeNoLabel])
            );
            let tmpdir = std::env::temp_dir();
            let dot_path = tmpdir.join("cycle.dot");
            std::fs::write(&dot_path, dot).ok()?;
            let lossy_path = dot_path.to_string_lossy();
            Some(lossy_path.into_owned())
        })
        .flatten();

    let cycle_lines = sccs
        .into_iter()
        .filter(|cycle| cycle.len() > 1)
        .map(|cycle| {
            let workspaces = cycle.into_iter().map(|id| graph.node_weight(id).unwrap());
            format!("\t{}", workspaces.format(", "))
        })
        .join("\n");

    if !cycle_lines.is_empty() {
        return Err(Error::CyclicDependencies { cycle_lines, dot });
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
        "###);
    }

    #[test]
    fn test_basic_cycle_break() {
        // Simple cycle where any edge would break the cycle
        let mut g = Graph::new();
        let a = g.add_node("a");
        let b = g.add_node("b");
        let c = g.add_node("c");

        let a_b = g.add_edge(a, b, ());
        let b_c = g.add_edge(b, c, ());
        let c_a = g.add_edge(c, a, ());

        let breaks = edges_to_break_cycle(&g);
        assert_eq!(breaks.len(), 3, "{:?}", breaks);
        let mut edges_that_break = HashSet::new();
        for brk in breaks {
            assert_eq!(brk.len(), 1);
            edges_that_break.extend(brk.into_iter());
        }
        assert_eq!(
            edges_that_break,
            [a_b, b_c, c_a].iter().copied().collect::<HashSet<_>>()
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
        let a_b = g.add_edge(a, b, ());
        let b_c = g.add_edge(b, c, ());
        let c_a = g.add_edge(c, a, ());

        // Cycle 2
        let b_d = g.add_edge(b, d, ());
        let d_a = g.add_edge(d, a, ());

        let breaks = edges_to_break_cycle(&g);
        assert_eq!(breaks.len(), 1, "{:?}", breaks);
        assert_eq!(breaks.into_iter().flatten().exactly_one().unwrap(), a_b);
    }

    #[test]
    fn test_cycle_break_two_edges() {
        // cycle where only 2 edges required to break the cycle
    }
}
