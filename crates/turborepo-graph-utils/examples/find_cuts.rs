use itertools::Itertools as _;
use petgraph::Graph;
use turborepo_graph_utils::cycles_and_cut_candidates;

fn main() {
    let size: usize = cli_size().unwrap_or(6);
    let g = generate_graph(size);
    let cycles = cycles_and_cut_candidates(&g);
    println!("found {} cycles", cycles.len());
    for (i, mut cycle) in cycles.into_iter().enumerate() {
        let cut_size = cycle.cuts.pop().unwrap_or_default().len();
        println!("cycle {i} needs {cut_size} cuts to be removed");
    }
}

fn cli_size() -> Option<usize> {
    std::env::args().nth(1)?.parse().ok()
}

// Generates a fully connected graph
fn generate_graph(size: usize) -> Graph<String, ()> {
    let mut g = Graph::new();
    let nodes = (0..size)
        .map(|i| g.add_node(i.to_string()))
        .collect::<Vec<_>>();

    for (s, d) in nodes.iter().cartesian_product(nodes.iter()) {
        if s != d {
            g.add_edge(*s, *d, ());
        }
    }

    g
}
