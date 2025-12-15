use std::{collections::HashMap, io};

use itertools::Itertools;
use petgraph::{visit::EdgeRef, Graph};
use rand::{distributions::Uniform, prelude::Distribution, Rng, SeedableRng};

use super::{Built, Engine, TaskNode};

struct CapitalLetters;

impl Distribution<char> for CapitalLetters {
    fn sample<R: rand::Rng + ?Sized>(&self, rng: &mut R) -> char {
        const RANGE: u32 = 26;
        const GEN_ASCII_STR_CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let range = Uniform::new(0u32, GEN_ASCII_STR_CHARSET.len() as u32);
        char::from_u32(GEN_ASCII_STR_CHARSET[range.sample(rng) as usize] as u32)
            .expect("random number should be in bounds")
    }
}

fn generate_id<R: Rng>(rng: &mut R) -> String {
    CapitalLetters.sample_iter(rng).take(4).join("")
}

impl Engine<Built> {
    pub fn mermaid_graph<W: io::Write>(&self, writer: W, is_single: bool) -> Result<(), io::Error> {
        render_graph(writer, &self.task_graph, is_single)
    }
}

fn render_graph<W: io::Write>(
    mut writer: W,
    graph: &Graph<TaskNode, ()>,
    is_single: bool,
) -> Result<(), io::Error> {
    // Chosen randomly.
    // Pick a constant seed so that the same graph generates the same nodes every
    // time. This is not a security-sensitive operation, it's just aliases for
    // the graph nodes.
    let mut rng = rand::rngs::SmallRng::seed_from_u64(4u64);

    let display_node = match is_single {
        true => |node: &TaskNode| match node {
            TaskNode::Root => node.to_string(),
            TaskNode::Task(task) => task.task().to_string(),
        },
        false => |node: &TaskNode| node.to_string(),
    };

    let mut edges = graph
        .edge_references()
        .map(|e| {
            (
                display_node(
                    graph
                        .node_weight(e.source())
                        .expect("node index should exist in graph"),
                ),
                display_node(
                    graph
                        .node_weight(e.target())
                        .expect("node index should exist in graph"),
                ),
            )
        })
        .collect::<Vec<_>>();
    edges.sort();

    writeln!(writer, "graph TD")?;
    let mut name_cache = HashMap::<String, String>::new();
    for (src, target) in edges {
        let src_name = name_cache
            .entry(src.clone())
            .or_insert_with(|| generate_id(&mut rng));
        write!(writer, "\t{src_name}(\"{src}\") --> ")?;
        let target_name = name_cache
            .entry(target.clone())
            .or_insert_with(|| generate_id(&mut rng));
        writeln!(writer, "{target_name}(\"{target}\")")?;
    }
    Ok(())
}
