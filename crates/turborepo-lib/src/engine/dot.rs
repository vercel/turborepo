use std::io;

use petgraph::{visit::EdgeRef, Graph};

use super::{Built, Engine, TaskNode};

impl Engine<Built> {
    pub fn dot_graph<W: io::Write>(&self, writer: W, is_single: bool) -> Result<(), io::Error> {
        let display_node = match is_single {
            true => |node: &TaskNode| match node {
                TaskNode::Root => node.to_string(),
                TaskNode::Task(task) => task.task().to_string(),
            },
            false => |node: &TaskNode| node.to_string(),
        };
        render_graph(&self.task_graph, display_node, writer)
    }
}

const GRAPH_PRELUDE: &str = "\ndigraph {\n\tcompound = \"true\"
\tnewrank = \"true\"
\tsubgraph \"root\" {
";

fn render_graph<N>(
    graph: &Graph<N, ()>,
    mut display_node: impl FnMut(&N) -> String,
    mut writer: impl io::Write,
) -> Result<(), io::Error> {
    let mut get_node = |i| {
        display_node(
            graph
                .node_weight(i)
                .expect("node index should exist in graph"),
        )
    };

    // These are hardcoded writes from the Go side that we just copy
    writer.write_all(GRAPH_PRELUDE.as_bytes())?;

    let mut edges = graph
        .edge_references()
        .map(|edge| {
            let source = get_node(edge.source());
            let target = get_node(edge.target());
            format!("\t\t\"[root] {source}\" -> \"[root] {target}\"")
        })
        .collect::<Vec<_>>();
    edges.sort();

    writer.write_all(edges.join("\n").as_bytes())?;

    writer.write_all("\n\t}\n}\n\n".as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_simple_graph_output() {
        let mut bytes = Vec::new();
        let mut graph = Graph::new();
        let root = graph.add_node("___ROOT___");
        let build = graph.add_node("build");
        graph.add_edge(root, build, ());
        render_graph(&graph, |n| n.to_string(), &mut bytes).unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "\ndigraph {
\tcompound = \"true\"
\tnewrank = \"true\"
\tsubgraph \"root\" {
\t\t\"[root] ___ROOT___\" -> \"[root] build\"
\t}
}\n\n"
        );
    }
}
