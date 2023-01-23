package core

import (
	"fmt"
	"strings"
	"testing"

	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/graph"
	"github.com/vercel/turbo/cli/internal/util"
	"gotest.tools/v3/assert"

	"github.com/pyr-sh/dag"
)

func testVisitor(taskID string) error {
	fmt.Println(taskID)
	return nil
}

func TestEngineDefault(t *testing.T) {
	var workspaceGraph dag.AcyclicGraph
	workspaceGraph.Add("a")
	workspaceGraph.Add("b")
	workspaceGraph.Add("c")
	workspaceGraph.Connect(dag.BasicEdge("c", "b"))
	workspaceGraph.Connect(dag.BasicEdge("c", "a"))

	buildTask := fs.TaskDefinition{
		TopologicalDependencies: []string{"build"},
		TaskDependencies:        []string{"prepare"},
	}

	testTask := fs.TaskDefinition{
		TopologicalDependencies: []string{"build"},
		TaskDependencies:        []string{"prepare"},
	}

	prepareTask := fs.TaskDefinition{}
	sideQuestTask := fs.TaskDefinition{TaskDependencies: []string{"prepare"}}

	pipeline := map[string]fs.TaskDefinition{
		"build":      buildTask,
		"test":       testTask,
		"prepare":    prepareTask,
		"side-quest": sideQuestTask,
	}

	p := NewEngine(&graph.CompleteGraph{
		WorkspaceGraph:  workspaceGraph,
		Pipeline:        pipeline,
		TaskDefinitions: map[string]*fs.TaskDefinition{},
		WorkspaceInfos: graph.WorkspaceInfos{
			"a": &fs.PackageJSON{},
			"b": &fs.PackageJSON{},
			"c": &fs.PackageJSON{},
		},
	})

	p.AddTask(&Task{Name: "build", TaskDefinition: buildTask})
	p.AddTask(&Task{Name: "test", TaskDefinition: testTask})
	p.AddTask(&Task{Name: "prepare"})
	p.AddTask(&Task{Name: "side-quest", TaskDefinition: sideQuestTask}) // not in the build/test tree

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

	errs := p.Execute(testVisitor, EngineExecutionOptions{
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
	g := dag.AcyclicGraph{}
	g.Add("a")
	g.Add("b")
	g.Add("c")
	graph := graph.CompleteGraph{WorkspaceGraph: g}
	p := NewEngine(&graph)

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
	workspaceGraph := dag.AcyclicGraph{}
	workspaceGraph.Add("app1")
	workspaceGraph.Add("app2")
	workspaceGraph.Add("libA")
	workspaceGraph.Add("libB")
	workspaceGraph.Add("libC")
	workspaceGraph.Add("libD")
	workspaceGraph.Connect(dag.BasicEdge("libA", "libB"))
	workspaceGraph.Connect(dag.BasicEdge("libB", "libD"))
	workspaceGraph.Connect(dag.BasicEdge("app0", "libA"))
	workspaceGraph.Connect(dag.BasicEdge("app1", "libA"))
	workspaceGraph.Connect(dag.BasicEdge("app2", "libB"))
	workspaceGraph.Connect(dag.BasicEdge("app2", "libC"))

	buildTask := fs.TaskDefinition{TopologicalDependencies: []string{"build"}}
	testTask := fs.TaskDefinition{TopologicalDependencies: []string{"build"}}

	pipeline := fs.Pipeline{"build": buildTask, "test": testTask}

	p := NewEngine(&graph.CompleteGraph{
		WorkspaceGraph:  workspaceGraph,
		Pipeline:        pipeline,
		TaskDefinitions: map[string]*fs.TaskDefinition{},
		WorkspaceInfos: graph.WorkspaceInfos{
			"app1": &fs.PackageJSON{},
			"app2": &fs.PackageJSON{},
			"libA": &fs.PackageJSON{},
			"libB": &fs.PackageJSON{},
			"libC": &fs.PackageJSON{},
			"libD": &fs.PackageJSON{},
		},
	})

	p.AddTask(&Task{Name: "build", TaskDefinition: buildTask})
	p.AddTask(&Task{Name: "test", TaskDefinition: testTask})

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
	errs := p.Execute(testVisitor, EngineExecutionOptions{
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

func TestRunPackageTask(t *testing.T) {
	workspaceGraph := dag.AcyclicGraph{}
	workspaceGraph.Add("app1")
	workspaceGraph.Add("libA")
	workspaceGraph.Connect(dag.BasicEdge("app1", "libA"))

	buildTask := fs.TaskDefinition{TopologicalDependencies: []string{"build"}}
	specialTask := fs.TaskDefinition{TopologicalDependencies: []string{"build"}}
	pipeline := fs.Pipeline{
		"build":        buildTask,
		"app1#special": specialTask,
	}

	p := NewEngine(&graph.CompleteGraph{
		WorkspaceGraph:  workspaceGraph,
		Pipeline:        pipeline,
		TaskDefinitions: map[string]*fs.TaskDefinition{},
		WorkspaceInfos: graph.WorkspaceInfos{
			"app1": &fs.PackageJSON{},
			"libA": &fs.PackageJSON{},
		},
	})

	p.AddTask(&Task{
		Name:           "app1#special",
		TaskDefinition: specialTask,
	})
	p.AddTask(&Task{
		Name:           "build",
		TaskDefinition: buildTask,
	})
	// equivalent to "turbo run special", without an entry for
	// "special" in turbo.json. Only "app1#special" is defined.
	err := p.Prepare(&EngineBuildingOptions{
		Packages:  []string{"app1", "libA"},
		TaskNames: []string{"special"},
	})
	assert.NilError(t, err, "Prepare")
	errs := p.Execute(testVisitor, EngineExecutionOptions{
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
	workspaceGraph := dag.AcyclicGraph{}
	workspaceGraph.Add("app")
	workspaceGraph.Add("lib")
	workspaceGraph.Connect(dag.BasicEdge("app", "lib"))

	p := NewEngine(&graph.CompleteGraph{WorkspaceGraph: workspaceGraph})

	err := p.Prepare(&EngineBuildingOptions{
		Packages:  []string{"app", "lib"},
		TaskNames: []string{"build"},
	})
	// should not fail because we have no tasks in the engine
	assert.NilError(t, err, "Prepare")
}

func TestIncludeRootTasks(t *testing.T) {
	workspaceGraph := dag.AcyclicGraph{}
	workspaceGraph.Add("app1")
	workspaceGraph.Add("libA")
	workspaceGraph.Connect(dag.BasicEdge("app1", "libA"))

	buildTask := fs.TaskDefinition{TopologicalDependencies: []string{"build"}}
	testTask := fs.TaskDefinition{TopologicalDependencies: []string{"build"}}
	rootTestTask := fs.TaskDefinition{}
	pipeline := fs.Pipeline{
		"build":   buildTask,
		"test":    testTask,
		"//#test": rootTestTask,
	}
	p := NewEngine(&graph.CompleteGraph{
		WorkspaceGraph:  workspaceGraph,
		Pipeline:        pipeline,
		TaskDefinitions: map[string]*fs.TaskDefinition{},
		WorkspaceInfos: graph.WorkspaceInfos{
			util.RootPkgName: &fs.PackageJSON{},
			"app1":           &fs.PackageJSON{},
			"libA":           &fs.PackageJSON{},
		},
	})

	p.AddTask(&Task{Name: "build", TaskDefinition: buildTask})
	p.AddTask(&Task{Name: "test", TaskDefinition: testTask})
	p.AddTask(&Task{Name: util.RootTaskID("test")})

	err := p.Prepare(&EngineBuildingOptions{
		Packages:  []string{util.RootPkgName, "app1", "libA"},
		TaskNames: []string{"build", "test"},
	})
	if err != nil {
		t.Fatalf("failed to prepare engine: %v", err)
	}
	errs := p.Execute(testVisitor, EngineExecutionOptions{
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
	workspaceGraph := dag.AcyclicGraph{}
	workspaceGraph.Add("app1")
	workspaceGraph.Add("libA")
	workspaceGraph.Connect(dag.BasicEdge("app1", "libA"))

	buildTask := fs.TaskDefinition{TopologicalDependencies: []string{"build"}}
	rootTask := fs.TaskDefinition{}

	pipeline := fs.Pipeline{
		"build":        buildTask,
		"//#root-task": rootTask,
	}

	p := NewEngine(&graph.CompleteGraph{
		WorkspaceGraph:  workspaceGraph,
		Pipeline:        pipeline,
		TaskDefinitions: map[string]*fs.TaskDefinition{},
		WorkspaceInfos: graph.WorkspaceInfos{
			util.RootPkgName: &fs.PackageJSON{},
			"app1":           &fs.PackageJSON{},
			"libA":           &fs.PackageJSON{},
		},
	})

	p.AddTask(&Task{Name: "build", TaskDefinition: buildTask})
	p.AddTask(&Task{Name: "//#root-task"})
	err := p.AddDep("//#root-task", "libA#build")
	assert.NilError(t, err, "AddDep")

	err = p.Prepare(&EngineBuildingOptions{
		Packages:  []string{"app1"},
		TaskNames: []string{"build"},
	})
	assert.NilError(t, err, "Prepare")
	errs := p.Execute(testVisitor, EngineExecutionOptions{
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
	workspaceGraph := dag.AcyclicGraph{}
	workspaceGraph.Add("app1")
	workspaceGraph.Add("libA")
	workspaceGraph.Connect(dag.BasicEdge("app1", "libA"))

	p := NewEngine(&graph.CompleteGraph{WorkspaceGraph: workspaceGraph})

	p.AddTask(&Task{
		Name:           "build",
		TaskDefinition: fs.TaskDefinition{TopologicalDependencies: []string{"build"}},
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

func TestDependOnMultiplePackageTasks(t *testing.T) {
	workspaceGraph := dag.AcyclicGraph{}
	workspaceGraph.Add("app1")
	workspaceGraph.Add("libA")
	workspaceGraph.Connect(dag.BasicEdge("app1", "libA"))

	buildTask := fs.TaskDefinition{TopologicalDependencies: []string{"build"}}
	compileTask := fs.TaskDefinition{TopologicalDependencies: []string{"build"}}

	pipeline := fs.Pipeline{
		"build":   buildTask,
		"compile": compileTask,
	}
	p := NewEngine(&graph.CompleteGraph{
		WorkspaceGraph:  workspaceGraph,
		Pipeline:        pipeline,
		TaskDefinitions: map[string]*fs.TaskDefinition{},
		WorkspaceInfos: graph.WorkspaceInfos{
			"app1": &fs.PackageJSON{},
			"libA": &fs.PackageJSON{},
		},
	})

	p.AddTask(&Task{Name: "build", TaskDefinition: buildTask})
	p.AddTask(&Task{Name: "compile", TaskDefinition: compileTask})

	err := p.AddDep("app1#build", "libA#build")
	assert.NilError(t, err, "AddDep")

	err = p.AddDep("app1#compile", "libA#build")
	assert.NilError(t, err, "AddDep")

	err = p.Prepare(&EngineBuildingOptions{
		Packages:  []string{"app1"},
		TaskNames: []string{"build"},
	})
	assert.NilError(t, err, "Prepare")

	actual := strings.TrimSpace(p.TaskGraph.String())
	expected := strings.TrimSpace(`
app1#build
  libA#build
app1#compile
  libA#build
libA#build
  app1#build
  app1#compile`)
	expected = strings.TrimSpace(expected)
	if actual != expected {
		t.Errorf("task graph got:\n%v\nwant:\n%v", actual, expected)
	}
}

func TestDependOnUnenabledRootTask(t *testing.T) {
	workspaceGraph := dag.AcyclicGraph{}
	workspaceGraph.Add("app1")
	workspaceGraph.Add("libA")
	workspaceGraph.Connect(dag.BasicEdge("app1", "libA"))

	p := NewEngine(&graph.CompleteGraph{WorkspaceGraph: workspaceGraph})

	p.AddTask(&Task{
		Name:           "build",
		TaskDefinition: fs.TaskDefinition{TopologicalDependencies: []string{"build"}},
	})
	p.AddTask(&Task{
		Name: "foo",
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
	var workspaceGraph dag.AcyclicGraph
	workspaceGraph.Add("a")
	workspaceGraph.Add("b")
	workspaceGraph.Add("c")
	workspaceGraph.Connect(dag.BasicEdge("c", "b"))
	workspaceGraph.Connect(dag.BasicEdge("c", "a"))

	buildTask := fs.TaskDefinition{
		TopologicalDependencies: []string{"build"},
		TaskDependencies:        []string{"prepare"},
	}
	testTask := fs.TaskDefinition{
		TopologicalDependencies: []string{"build"},
		TaskDependencies:        []string{"prepare"},
	}
	prepareTask := fs.TaskDefinition{}

	pipeline := map[string]fs.TaskDefinition{
		"build":   buildTask,
		"test":    testTask,
		"prepare": prepareTask,
	}

	p := NewEngine(&graph.CompleteGraph{
		WorkspaceGraph:  workspaceGraph,
		Pipeline:        pipeline,
		TaskDefinitions: map[string]*fs.TaskDefinition{},
		WorkspaceInfos: graph.WorkspaceInfos{
			"a": &fs.PackageJSON{},
			"b": &fs.PackageJSON{},
			"c": &fs.PackageJSON{},
		},
	})

	p.AddTask(&Task{
		Name:           "build",
		TaskDefinition: buildTask,
	})
	p.AddTask(&Task{
		Name:           "test",
		TaskDefinition: testTask,
	})
	p.AddTask(&Task{
		Name:           "prepare",
		TaskDefinition: prepareTask,
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

	errs := p.Execute(testVisitor, EngineExecutionOptions{
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
