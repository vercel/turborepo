use petgraph::{algo::kosaraju_scc, prelude::*, stable_graph::IndexType, EdgeType, Graph};

/// Port of [petgraph::algo::condensation] for our use case.
pub fn condensation<N, E, Ty, Ix>(
    g: Graph<N, E, Ty, Ix>,
    make_acyclic: bool,
) -> Graph<Vec<N>, E, Ty, Ix>
where
    Ty: EdgeType,
    Ix: IndexType,
{
    let sccs = kosaraju_scc(&g);
    let mut condensed: Graph<Vec<N>, E, Ty, Ix> = Graph::with_capacity(sccs.len(), g.edge_count());

    // Build a map from old indices to new ones.
    let mut node_map = vec![NodeIndex::end(); g.node_count()];
    for comp in sccs {
        let new_nix = condensed.add_node(Vec::new());
        for nix in comp {
            node_map[nix.index()] = new_nix;
        }
    }

    // Consume nodes and edges of the old graph and insert them into the new one.
    let (nodes, edges) = g.into_nodes_edges();
    for (nix, node) in nodes.into_iter().enumerate() {
        condensed[node_map[nix]].push(node.weight);
    }
    for edge in edges {
        let source = node_map[edge.source().index()];
        let target = node_map[edge.target().index()];
        if make_acyclic {
            if source != target {
                condensed.update_edge(source, target, edge.weight);
            }
        } else {
            condensed.add_edge(source, target, edge.weight);
        }
    }
    condensed
}
