package run

import (
	gocontext "context"

	"github.com/pyr-sh/dag"
	"github.com/vercel/turbo/cli/internal/cmdutil"
	"github.com/vercel/turbo/cli/internal/core"
	"github.com/vercel/turbo/cli/internal/graphvisualizer"
	"github.com/vercel/turbo/cli/internal/util"
)

// GraphRun generates a visualization of the task graph rather than executing it.
func GraphRun(ctx gocontext.Context, rs *runSpec, engine *core.Engine, base *cmdutil.CmdBase) error {
	graph := engine.TaskGraph
	if rs.Opts.runOpts.SinglePackage {
		graph = filterSinglePackageGraphForDisplay(engine.TaskGraph)
	}
	visualizer := graphvisualizer.New(base.RepoRoot, base.UI, graph)

	if rs.Opts.runOpts.GraphDot {
		visualizer.RenderDotGraph()
	} else {
		err := visualizer.GenerateGraphFile(rs.Opts.runOpts.GraphFile)
		if err != nil {
			return err
		}
	}
	return nil
}

// filterSinglePackageGraphForDisplay builds an equivalent graph with package names stripped from tasks.
// Given that this should only be used in a single-package context, all of the package names are expected
// to be //. Also, all nodes are always connected to the root node, so we are not concerned with leaving
// behind any unconnected nodes.
func filterSinglePackageGraphForDisplay(originalGraph *dag.AcyclicGraph) *dag.AcyclicGraph {
	graph := &dag.AcyclicGraph{}
	for _, edge := range originalGraph.Edges() {
		src := util.StripPackageName(edge.Source().(string))
		tgt := util.StripPackageName(edge.Target().(string))
		graph.Add(src)
		graph.Add(tgt)
		graph.Connect(dag.BasicEdge(src, tgt))
	}
	return graph
}
