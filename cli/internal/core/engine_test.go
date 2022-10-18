package core

import (
	"fmt"
	"strings"
	"testing"

	"github.com/pyr-sh/dag"
	assertOther "github.com/stretchr/testify/assert"
	"github.com/vercel/turbo/cli/internal/util"
	"gotest.tools/v3/assert"
)

func testVisitor(taskID string) error {
	fmt.Println(taskID)
	return nil
}

func TestEngineDefault(t *testing.T) {
	var g dag.AcyclicGraph
	g.Add("a")
	g.Add("b")
	g.Add("c")
	g.Connect(dag.BasicEdge("c", "b"))
	g.Connect(dag.BasicEdge("c", "a"))

	p := NewEngine(&g)
	topoDeps := make(util.Set)
	topoDeps.Add("build")
	deps := make(util.Set)
	deps.Add("prepare")
	p.AddTask(&Task{
		Name:     "build",
		TopoDeps: topoDeps,
		Deps:     deps,
	})
	p.AddTask(&Task{
		Name:     "test",
		TopoDeps: topoDeps,
		Deps:     deps,
	})
	p.AddTask(&Task{
		Name: "prepare",
	})
	p.AddTask(&Task{
		Name: "side-quest", // not in the build/test tree
		Deps: deps,
	})

	if _, ok := p.Tasks["build"]; !ok {
		t.Fatal("AddTask is not adding tasks (build)")
	}

	if _, ok := p.Tasks["test"]; !ok {
		t.Fatal("AddTask is not adding tasks (test)")
	}

	err := p.Prepare(&EngineBuildingOptions{
		Packages:  []string{"a", "b", "c"},
		TaskNames: []string{"test"},
		TasksOnly: false,
	})

	if err != nil {
		t.Fatalf("%v", err)
	}

	errs := p.Execute(testVisitor, EngineBuildingOptions{
		Concurrency: 10,
	})

	for _, err := range errs {
		t.Fatalf("%v", err)
	}

	actual := strings.TrimSpace(p.TaskGraph.String())
	expected := strings.TrimSpace(leafStringAll)
	if actual != expected {
		t.Fatalf("bad: \n\nactual---\n%s\n\n expected---\n%s", actual, expected)
	}
}

func TestUnknownDependency(t *testing.T) {
	g := &dag.AcyclicGraph{}
	g.Add("a")
	g.Add("b")
	g.Add("c")
	p := NewEngine(g)
	err := p.AddDep("unknown#custom", "build")
	if err == nil {
		t.Error("expected error for unknown package, got nil")
	}
	err = p.AddDep("a#custom", "build")
	if err != nil {
		t.Errorf("expected no error for package task with known package, got %v", err)
	}
}

func TestDependenciesOnUnspecifiedPackages(t *testing.T) {
	// app1 -> libA
	//              \
	//                > libB -> libD
	//              /
	//       app2 <
	//              \ libC
	//
	graph := &dag.AcyclicGraph{}
	graph.Add("app1")
	graph.Add("app2")
	graph.Add("libA")
	graph.Add("libB")
	graph.Add("libC")
	graph.Add("libD")
	graph.Connect(dag.BasicEdge("libA", "libB"))
	graph.Connect(dag.BasicEdge("libB", "libD"))
	graph.Connect(dag.BasicEdge("app0", "libA"))
	graph.Connect(dag.BasicEdge("app1", "libA"))
	graph.Connect(dag.BasicEdge("app2", "libB"))
	graph.Connect(dag.BasicEdge("app2", "libC"))

	p := NewEngine(graph)
	dependOnBuild := make(util.Set)
	dependOnBuild.Add("build")
	p.AddTask(&Task{
		Name:     "build",
		TopoDeps: dependOnBuild,
		Deps:     make(util.Set),
	})
	p.AddTask(&Task{
		Name:     "test",
		TopoDeps: dependOnBuild,
		Deps:     make(util.Set),
	})
	// We're only requesting one package ("scope"),
	// but the combination of that package and task causes
	// dependencies to also get run. This is the equivalent of
	// turbo run test --filter=app2
	err := p.Prepare(&EngineBuildingOptions{
		Packages:  []string{"app2"},
		TaskNames: []string{"test"},
	})
	if err != nil {
		t.Fatalf("failed to prepare engine: %v", err)
	}
	errs := p.Execute(testVisitor, EngineBuildingOptions{
		Concurrency: 10,
	})
	for _, err := range errs {
		t.Fatalf("error executing tasks: %v", err)
	}
	expected := `
___ROOT___
app2#test
  libB#build
  libC#build
libB#build
  libD#build
libC#build
  ___ROOT___
libD#build
  ___ROOT___
`
	expected = strings.TrimSpace(expected)
	actual := strings.TrimSpace(p.TaskGraph.String())
	if actual != expected {
		t.Errorf("task graph got:\n%v\nwant:\n%v", actual, expected)
	}
}

func TestPrepare_PersistentDependencies_Topological(t *testing.T) {
	pkgGraph, workspaces := _buildWorkspaceGraph()
	engine := NewEngine(&pkgGraph)

	// "dev": dependsOn: ["^dev"] (where dev is persistent)
	engine.AddTask(&Task{
		Name:       "dev",
		TopoDeps:   util.SetFromStrings([]string{"dev"}),
		Deps:       make(util.Set), // empty, no non-caret task deps.
		Persistent: true,
	})

	err := engine.Prepare(&EngineBuildingOptions{
		Packages:  workspaces,
		TaskNames: []string{"dev"},
		TasksOnly: false,
	})

	expected := "Persistent tasks cannot depend on other persistent tasks. Found \"workspace-c#dev\" depends on \"workspace-b#dev\""
	assertOther.EqualErrorf(t, err, expected, "")
}

func TestPrepare_PersistentDependencies_SameWorkspace(t *testing.T) {
	pkgGraph, workspaces := _buildWorkspaceGraph()
	engine := NewEngine(&pkgGraph)

	// "build": dependsOn: ["something"] (where build is not, but "something" is persistent)
	engine.AddTask(&Task{
		Name:       "build",
		TopoDeps:   make(util.Set), // empty
		Deps:       util.SetFromStrings([]string{"something"}),
		Persistent: false,
	})

	engine.AddTask(&Task{
		Name:       "something",
		TopoDeps:   make(util.Set),
		Deps:       make(util.Set),
		Persistent: true,
	})

	err := engine.Prepare(&EngineBuildingOptions{
		Packages:  workspaces,
		TaskNames: []string{"build"},
		TasksOnly: false,
	})

	expected := "Persistent tasks cannot depend on other persistent tasks. Found \"workspace-a#build\" depends on \"workspace-a#something\""
	assertOther.EqualErrorf(t, err, expected, "")
}

func TestPrepare_PersistentDependencies_WorkspaceSpecific(t *testing.T) {
	pkgGraph, workspaces := _buildWorkspaceGraph()
	engine := NewEngine(&pkgGraph)

	// "dev": dependsOn: ["workspace-b#something"]
	engine.AddTask(&Task{
		Name:       "dev",
		TopoDeps:   make(util.Set), // empty
		Deps:       util.SetFromStrings([]string{"workspace-b#something"}),
		Persistent: true,
	})

	// workspace-b#something is persistent, and has no dependencies
	engine.AddTask(&Task{
		Name:       "workspace-b#something",
		TopoDeps:   make(util.Set), // empty
		Deps:       make(util.Set), // empty
		Persistent: true,
	})

	// Do the thing
	err := engine.Prepare(&EngineBuildingOptions{
		Packages:  workspaces,
		TaskNames: []string{"dev"},
		TasksOnly: false,
	})

	expected := "Persistent tasks cannot depend on other persistent tasks. Found \"workspace-a#dev\" depends on \"workspace-b#something\""
	assertOther.EqualErrorf(t, err, expected, "")
}

func TestPrepare_PersistentDependencies_CrossWorkspace(t *testing.T) {
	// "workspace-a#dev": dependsOn: ["workspace-b#dev"] (where workspace-a#dev and workspace-b#dev are persistent)

	pkgGraph, workspaces := _buildWorkspaceGraph()
	engine := NewEngine(&pkgGraph)

	// workspace-a#dev dependsOn workspace-a#dev
	if err := engine.AddDep("workspace-b#dev", "workspace-a#dev"); err != nil {
		t.Fatalf("Something went wrong in test construction: %s", err)
	}

	// workspace-a#dev specifically dependsOn workspace-b#dev
	engine.AddTask(&Task{
		Name:       "workspace-a#dev",
		TopoDeps:   make(util.Set), // empty
		Deps:       util.SetFromStrings([]string{"workspace-b#dev"}),
		Persistent: true,
	})

	// workspace-b#dev dependsOn nothing else
	engine.AddTask(&Task{
		Name:       "workspace-b#dev",
		TopoDeps:   make(util.Set), // empty
		Deps:       make(util.Set), // empty
		Persistent: true,
	})

	err := engine.Prepare(&EngineBuildingOptions{
		Packages:  workspaces,
		TaskNames: []string{"dev"},
		TasksOnly: false,
	})

	expected := "Persistent tasks cannot depend on other persistent tasks. Found \"workspace-a#dev\" depends on \"workspace-b#dev\""
	assertOther.EqualErrorf(t, err, expected, "")
}

func TestRunPackageTask(t *testing.T) {
	graph := &dag.AcyclicGraph{}
	graph.Add("app1")
	graph.Add("libA")
	graph.Connect(dag.BasicEdge("app1", "libA"))

	p := NewEngine(graph)
	dependOnBuild := make(util.Set)
	dependOnBuild.Add("build")
	p.AddTask(&Task{
		Name:     "app1#special",
		TopoDeps: dependOnBuild,
		Deps:     make(util.Set),
	})
	p.AddTask(&Task{
		Name:     "build",
		TopoDeps: dependOnBuild,
		Deps:     make(util.Set),
	})
	// equivalent to "turbo run special", without an entry for
	// "special" in turbo.json. Only "app1#special" is defined.
	err := p.Prepare(&EngineBuildingOptions{
		Packages:  []string{"app1", "libA"},
		TaskNames: []string{"special"},
	})
	assert.NilError(t, err, "Prepare")
	errs := p.Execute(testVisitor, EngineBuildingOptions{
		Concurrency: 10,
	})
	for _, err := range errs {
		assert.NilError(t, err, "Execute")
	}
	actual := strings.TrimSpace(p.TaskGraph.String())
	expected := strings.TrimSpace(`
___ROOT___
app1#special
  libA#build
libA#build
  ___ROOT___`)
	assert.Equal(t, expected, actual)
}

func TestRunWithNoTasksFound(t *testing.T) {
	graph := &dag.AcyclicGraph{}
	graph.Add("app")
	graph.Add("lib")
	graph.Connect(dag.BasicEdge("app", "lib"))

	p := NewEngine(graph)
	dependOnBuild := make(util.Set)
	dependOnBuild.Add("build")

	err := p.Prepare(&EngineBuildingOptions{
		Packages:  []string{"app", "lib"},
		TaskNames: []string{"build"},
	})
	// should not fail because we have no tasks in the engine
	assert.NilError(t, err, "Prepare")
}

func TestIncludeRootTasks(t *testing.T) {
	graph := &dag.AcyclicGraph{}
	graph.Add("app1")
	graph.Add("libA")
	graph.Connect(dag.BasicEdge("app1", "libA"))

	p := NewEngine(graph)
	dependOnBuild := make(util.Set)
	dependOnBuild.Add("build")
	p.AddTask(&Task{
		Name:     "build",
		TopoDeps: dependOnBuild,
		Deps:     make(util.Set),
	})
	p.AddTask(&Task{
		Name:     "test",
		TopoDeps: dependOnBuild,
		Deps:     make(util.Set),
	})
	p.AddTask(&Task{
		Name:     util.RootTaskID("test"),
		TopoDeps: make(util.Set),
		Deps:     make(util.Set),
	})
	err := p.Prepare(&EngineBuildingOptions{
		Packages:  []string{util.RootPkgName, "app1", "libA"},
		TaskNames: []string{"build", "test"},
	})
	if err != nil {
		t.Fatalf("failed to prepare engine: %v", err)
	}
	errs := p.Execute(testVisitor, EngineBuildingOptions{
		Concurrency: 10,
	})
	for _, err := range errs {
		t.Fatalf("error executing tasks: %v", err)
	}
	actual := strings.TrimSpace(p.TaskGraph.String())
	expected := fmt.Sprintf(`
%v#test
  ___ROOT___
___ROOT___
app1#build
  libA#build
app1#test
  libA#build
libA#build
  ___ROOT___
libA#test
  ___ROOT___
`, util.RootPkgName)
	expected = strings.TrimSpace(expected)
	if actual != expected {
		t.Errorf("task graph got:\n%v\nwant:\n%v", actual, expected)
	}
}

func TestDependOnRootTask(t *testing.T) {
	graph := &dag.AcyclicGraph{}
	graph.Add("app1")
	graph.Add("libA")
	graph.Connect(dag.BasicEdge("app1", "libA"))

	p := NewEngine(graph)
	dependOnBuild := make(util.Set)
	dependOnBuild.Add("build")

	p.AddTask(&Task{
		Name:     "build",
		TopoDeps: dependOnBuild,
		Deps:     make(util.Set),
	})
	p.AddTask(&Task{
		Name:     "//#root-task",
		TopoDeps: make(util.Set),
		Deps:     make(util.Set),
	})
	err := p.AddDep("//#root-task", "libA#build")
	assert.NilError(t, err, "AddDep")

	err = p.Prepare(&EngineBuildingOptions{
		Packages:  []string{"app1"},
		TaskNames: []string{"build"},
	})
	assert.NilError(t, err, "Prepare")
	errs := p.Execute(testVisitor, EngineBuildingOptions{
		Concurrency: 10,
	})
	for _, err := range errs {
		assert.NilError(t, err, "Execute")
	}
	actual := strings.TrimSpace(p.TaskGraph.String())
	expected := fmt.Sprintf(`%v#root-task
  ___ROOT___
___ROOT___
app1#build
  libA#build
libA#build
  %v#root-task`, util.RootPkgName, util.RootPkgName)
	assert.Equal(t, expected, actual)
}

func TestDependOnMissingRootTask(t *testing.T) {
	graph := &dag.AcyclicGraph{}
	graph.Add("app1")
	graph.Add("libA")
	graph.Connect(dag.BasicEdge("app1", "libA"))

	p := NewEngine(graph)
	dependOnBuild := make(util.Set)
	dependOnBuild.Add("build")

	p.AddTask(&Task{
		Name:     "build",
		TopoDeps: dependOnBuild,
		Deps:     make(util.Set),
	})
	err := p.AddDep("//#root-task", "libA#build")
	assert.NilError(t, err, "AddDep")

	err = p.Prepare(&EngineBuildingOptions{
		Packages:  []string{"app1"},
		TaskNames: []string{"build"},
	})
	if err == nil {
		t.Error("expected an error depending on non-existent root task")
	}
}

func TestDependOnUnenabledRootTask(t *testing.T) {
	graph := &dag.AcyclicGraph{}
	graph.Add("app1")
	graph.Add("libA")
	graph.Connect(dag.BasicEdge("app1", "libA"))

	p := NewEngine(graph)
	dependOnBuild := make(util.Set)
	dependOnBuild.Add("build")

	p.AddTask(&Task{
		Name:     "build",
		TopoDeps: dependOnBuild,
		Deps:     make(util.Set),
	})
	p.AddTask(&Task{
		Name:     "foo",
		TopoDeps: make(util.Set),
		Deps:     make(util.Set),
	})
	err := p.AddDep("//#foo", "libA#build")
	assert.NilError(t, err, "AddDep")

	err = p.Prepare(&EngineBuildingOptions{
		Packages:  []string{"app1"},
		TaskNames: []string{"build"},
	})
	if err == nil {
		t.Error("expected an error depending on un-enabled root task")
	}
}

func TestEngineTasksOnly(t *testing.T) {
	var g dag.AcyclicGraph
	g.Add("a")
	g.Add("b")
	g.Add("c")
	g.Connect(dag.BasicEdge("c", "b"))
	g.Connect(dag.BasicEdge("c", "a"))

	p := NewEngine(&g)
	topoDeps := make(util.Set)
	topoDeps.Add("build")
	deps := make(util.Set)
	deps.Add("prepare")
	p.AddTask(&Task{
		Name:     "build",
		TopoDeps: topoDeps,
		Deps:     deps,
	})
	p.AddTask(&Task{
		Name:     "test",
		TopoDeps: topoDeps,
		Deps:     deps,
	})
	p.AddTask(&Task{
		Name: "prepare",
	})

	if _, ok := p.Tasks["build"]; !ok {
		t.Fatal("AddTask is not adding tasks (build)")
	}

	if _, ok := p.Tasks["test"]; !ok {
		t.Fatal("AddTask is not adding tasks (test)")
	}

	err := p.Prepare(&EngineBuildingOptions{
		Packages:  []string{"a", "b", "c"},
		TaskNames: []string{"test"},
		TasksOnly: true,
	})

	if err != nil {
		t.Fatalf("%v", err)
	}

	errs := p.Execute(testVisitor, EngineBuildingOptions{
		Concurrency: 10,
	})

	for _, err := range errs {
		t.Fatalf("%v", err)
	}

	actual := strings.TrimSpace(p.TaskGraph.String())
	expected := strings.TrimSpace(leafStringOnly)
	if actual != expected {
		t.Fatalf("bad: \n\nactual---\n%s\n\n expected---\n%s", actual, expected)
	}
}

const leafStringAll = `
___ROOT___
a#build
  a#prepare
a#prepare
  ___ROOT___
a#test
  a#prepare
b#build
  b#prepare
b#prepare
  ___ROOT___
b#test
  b#prepare
c#prepare
  ___ROOT___
c#test
  a#build
  b#build
  c#prepare
`

const leafStringOnly = `
___ROOT___
a#test
  ___ROOT___
b#test
  ___ROOT___
c#test
  ___ROOT___
`

// helper function for some of the tests to set up workspace
func _buildWorkspaceGraph() (dag.AcyclicGraph, []string) {
	var graph dag.AcyclicGraph
	// build workspace graph.
	allWorkspaces := []string{"workspace-a", "workspace-b", "workspace-c"}
	for _, workspace := range allWorkspaces {
		graph.Add(workspace)
	}

	// A and B depend on C.
	graph.Connect(dag.BasicEdge("workspace-c", "workspace-b"))
	graph.Connect(dag.BasicEdge("workspace-c", "workspace-a"))
	return graph, allWorkspaces
}
