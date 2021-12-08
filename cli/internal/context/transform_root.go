package context

import "github.com/pyr-sh/dag"

// RootTransformer is a GraphTransformer that adds a root to the graph.
type RootTransformer struct{}

func (t *RootTransformer) Transform(g *dag.AcyclicGraph) error {
	// If we already have a good root, we're done
	if _, err := g.Root(); err == nil {
		return nil
	}

	// Add a root

	g.Add(ROOT_NODE_NAME)

	// Connect the root to all the edges that need it
	for _, v := range g.Vertices() {
		if v == ROOT_NODE_NAME {
			continue
		}

		if g.UpEdges(v).Len() == 0 {
			g.Connect(dag.BasicEdge(ROOT_NODE_NAME, v))
		}
	}

	return nil
}
