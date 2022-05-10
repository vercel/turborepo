package util

import (
	"fmt"
	"strings"

	"github.com/pyr-sh/dag"
)

// ValidateGraph checks that a given DAG has no cycles and no self-referential edges.
// We differ from the underlying DAG Validate method in that we allow multiple roots.
func ValidateGraph(graph *dag.AcyclicGraph) error {
	// We use Cycles instead of Validate because
	// our DAG has multiple roots (entrypoints).
	// Validate mandates that there is only a single root node.
	cycles := graph.Cycles()
	if len(cycles) > 0 {
		cycleLines := make([]string, len(cycles))
		for i, cycle := range cycles {
			vertices := make([]string, len(cycle))
			for j, vertex := range cycle {
				vertices[j] = vertex.(string)
			}
			cycleLines[i] = "\t" + strings.Join(vertices, ",")
		}
		return fmt.Errorf("cyclic dependency detected:\n%s", strings.Join(cycleLines, "\n"))
	}

	for _, e := range graph.Edges() {
		if e.Source() == e.Target() {
			return fmt.Errorf("%s depends on itself", e.Source())
		}
	}
	return nil
}
