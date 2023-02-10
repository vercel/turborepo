package graphvisualizer

import (
	"fmt"
	"io"
	"math/rand"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"

	"github.com/fatih/color"
	"github.com/mitchellh/cli"
	"github.com/pyr-sh/dag"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/ui"
	"github.com/vercel/turbo/cli/internal/util"
	"github.com/vercel/turbo/cli/internal/util/browser"
)

// GraphVisualizer requirements
type GraphVisualizer struct {
	repoRoot  turbopath.AbsoluteSystemPath
	ui        cli.Ui
	TaskGraph *dag.AcyclicGraph
}

// hasGraphViz checks for the presence of https://graphviz.org/
func hasGraphViz() bool {
	err := exec.Command("dot", "-V").Run()
	return err == nil
}

func getRandChar() string {
	i := rand.Intn(25) + 65
	return string(rune(i))
}

func getRandID() string {
	return getRandChar() + getRandChar() + getRandChar() + getRandChar()
}

// New creates an instance of ColorCache with helpers for adding colors to task outputs
func New(repoRoot turbopath.AbsoluteSystemPath, ui cli.Ui, TaskGraph *dag.AcyclicGraph) *GraphVisualizer {
	return &GraphVisualizer{
		repoRoot:  repoRoot,
		ui:        ui,
		TaskGraph: TaskGraph,
	}
}

// Converts the TaskGraph dag into a string
func (g *GraphVisualizer) generateDotString() string {
	return string(g.TaskGraph.Dot(&dag.DotOpts{
		Verbose:    true,
		DrawCycles: true,
	}))
}

// Outputs a warning when a file was requested, but graphviz is not available
func (g *GraphVisualizer) graphVizWarnUI() {
	g.ui.Warn(color.New(color.FgYellow, color.Bold, color.ReverseVideo).Sprint(" WARNING ") + color.YellowString(" `turbo` uses Graphviz to generate an image of your\ngraph, but Graphviz isn't installed on this machine.\n\nYou can download Graphviz from https://graphviz.org/download.\n\nIn the meantime, you can use this string output with an\nonline Dot graph viewer."))
}

// RenderDotGraph renders a dot graph string for the current TaskGraph
func (g *GraphVisualizer) RenderDotGraph() {
	g.ui.Output("")
	g.ui.Output(g.generateDotString())
}

type nameCache map[string]string

func (nc nameCache) getName(in string) string {
	if existing, ok := nc[in]; ok {
		return existing
	}
	newName := getRandID()
	nc[in] = newName
	return newName
}

type sortableEdge dag.Edge
type sortableEdges []sortableEdge

// methods mostly copied from marshalEdges in the dag library
func (e sortableEdges) Less(i, j int) bool {
	iSrc := dag.VertexName(e[i].Source())
	jSrc := dag.VertexName(e[j].Source())
	if iSrc < jSrc {
		return true
	} else if iSrc > jSrc {
		return false
	}
	return dag.VertexName(e[i].Target()) < dag.VertexName(e[j].Target())
}
func (e sortableEdges) Len() int      { return len(e) }
func (e sortableEdges) Swap(i, j int) { e[i], e[j] = e[j], e[i] }

func (g *GraphVisualizer) generateMermaid(out io.StringWriter) error {
	if _, err := out.WriteString("graph TD\n"); err != nil {
		return err
	}
	cache := make(nameCache)
	// cast edges to our custom type so we can sort them
	// this allows us to generate the same graph every time
	var edges sortableEdges
	for _, edge := range g.TaskGraph.Edges() {
		edges = append(edges, sortableEdge(edge))
	}
	sort.Sort(edges)
	for _, edge := range edges {
		left := dag.VertexName(edge.Source())
		right := dag.VertexName(edge.Target())
		leftName := cache.getName(left)
		rightName := cache.getName(right)
		if _, err := out.WriteString(fmt.Sprintf("\t%v(\"%v\") --> %v(\"%v\")\n", leftName, left, rightName, right)); err != nil {
			return err
		}
	}
	return nil
}

// GenerateGraphFile saves a visualization of the TaskGraph to a file (or renders a DotGraph as a fallback))
func (g *GraphVisualizer) GenerateGraphFile(outputName string) error {
	outputFilename := g.repoRoot.UntypedJoin(outputName)
	ext := outputFilename.Ext()
	// use .jpg as default extension if none is provided
	if ext == "" {
		ext = ".jpg"
		outputFilename = g.repoRoot.UntypedJoin(outputName + ext)
	}
	if ext == ".mermaid" {
		f, err := outputFilename.Create()
		if err != nil {
			return fmt.Errorf("error creating file: %w", err)
		}
		defer util.CloseAndIgnoreError(f)
		if err := g.generateMermaid(f); err != nil {
			return err
		}
		g.ui.Output(fmt.Sprintf("✔ Generated task graph in %s", ui.Bold(outputFilename.ToString())))
		return nil
	}
	graphString := g.generateDotString()
	if ext == ".html" {
		f, err := outputFilename.Create()
		if err != nil {
			return fmt.Errorf("error creating file: %w", err)
		}
		defer f.Close() //nolint errcheck
		_, writeErr1 := f.WriteString(`<!DOCTYPE html>
    <html>
    <head>
      <meta charset="utf-8">
      <title>Graph</title>
    </head>
    <body>
      <script src="https://cdn.jsdelivr.net/npm/viz.js@2.1.2-pre.1/viz.js"></script>
      <script src="https://cdn.jsdelivr.net/npm/viz.js@2.1.2-pre.1/full.render.js"></script>
      <script>`)
		if writeErr1 != nil {
			return fmt.Errorf("error writing graph contents: %w", writeErr1)
		}

		_, writeErr2 := f.WriteString("const s = `" + graphString + "`.replace(/\\_\\_\\_ROOT\\_\\_\\_/g, \"Root\").replace(/\\[root\\]/g, \"\");new Viz().renderSVGElement(s).then(el => document.body.appendChild(el)).catch(e => console.error(e));")
		if writeErr2 != nil {
			return fmt.Errorf("error creating file: %w", writeErr2)
		}

		_, writeErr3 := f.WriteString(`
    </script>
  </body>
  </html>`)
		if writeErr3 != nil {
			return fmt.Errorf("error creating file: %w", writeErr3)
		}

		g.ui.Output("")
		g.ui.Output(fmt.Sprintf("✔ Generated task graph in %s", ui.Bold(outputFilename.ToString())))
		if ui.IsTTY {
			if err := browser.OpenBrowser(outputFilename.ToString()); err != nil {
				g.ui.Warn(color.New(color.FgYellow, color.Bold, color.ReverseVideo).Sprintf("failed to open browser. Please navigate to file://%v", filepath.ToSlash(outputFilename.ToString())))
			}
		}
		return nil
	}
	hasDot := hasGraphViz()
	if hasDot {
		dotArgs := []string{"-T" + ext[1:], "-o", outputFilename.ToString()}
		cmd := exec.Command("dot", dotArgs...)
		cmd.Stdin = strings.NewReader(graphString)
		if err := cmd.Run(); err != nil {
			return fmt.Errorf("could not generate task graphfile %v:  %w", outputFilename, err)
		}
		g.ui.Output("")
		g.ui.Output(fmt.Sprintf("✔ Generated task graph in %s", ui.Bold(outputFilename.ToString())))

	} else {
		g.ui.Output("")
		// User requested a file, but we're falling back to console here so warn about installing graphViz correctly
		g.graphVizWarnUI()
		g.RenderDotGraph()
	}
	return nil
}
