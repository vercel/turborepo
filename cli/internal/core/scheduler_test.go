package core

import (
	"fmt"
	"strings"
	"testing"

	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/util"

	"github.com/pyr-sh/dag"
)

func testVisitor(taskID string) error {
	fmt.Println(taskID)
	return nil
}

func TestSchedulerDefault(t *testing.T) {
	var g dag.AcyclicGraph
	g.Add("a")
	g.Add("b")
	g.Add("c")
	g.Connect(dag.BasicEdge("c", "b"))
	g.Connect(dag.BasicEdge("c", "a"))

	p := NewScheduler(&g, map[interface{}]*fs.PackageJSON{
		"a": {
			Scripts: map[string]string{
				"build":   "build-cmd",
				"test":    "test-cmd",
				"prepare": "prepare-cmd",
			},
		},
		"b": {
			Scripts: map[string]string{
				"build":   "build-cmd",
				"test":    "test-cmd",
				"prepare": "prepare-cmd",
			},
		},
		"c": {
			Scripts: map[string]string{
				"build":   "build-cmd",
				"test":    "test-cmd",
				"prepare": "prepare-cmd",
			},
		},
	})
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

	err := p.Prepare(&SchedulerExecutionOptions{
		Packages:  []string{"a", "b", "c"},
		TaskNames: []string{"test"},
		TasksOnly: false,
	})

	if err != nil {
		t.Fatalf("%v", err)
	}

	errs := p.Execute(testVisitor, ExecOpts{
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

func TestSchedulerIgnoreNonexistent(t *testing.T) {
	testCases := []struct {
		Name     string
		Packages []string
		Tasks    []string
		Expected string
	}{
		{
			"test c",
			[]string{"c"},
			[]string{"test"},
			`___ROOT___
a#build
  ___ROOT___
b#build
  ___ROOT___
c#test
  a#build
  b#build
`,
		},
		{
			"build c",
			[]string{"c"},
			[]string{"build"},
			"",
		},
		{
			"build all",
			[]string{"a", "b", "c"},
			[]string{"build"},
			`___ROOT___
a#build
  ___ROOT___
b#build
  ___ROOT___
`,
		},
		{
			"test all",
			[]string{"a", "b", "c"},
			[]string{"test"},
			`___ROOT___
a#build
  ___ROOT___
b#build
  ___ROOT___
b#test
  ___ROOT___
c#test
  a#build
  b#build
`,
		},
	}
	for _, tc := range testCases {
		t.Run(tc.Name, func(t *testing.T) {
			var g dag.AcyclicGraph
			g.Add("a")
			g.Add("b")
			g.Add("c")
			g.Connect(dag.BasicEdge("c", "b"))
			g.Connect(dag.BasicEdge("c", "a"))

			p := NewScheduler(&g, map[interface{}]*fs.PackageJSON{
				"a": {
					Scripts: map[string]string{
						"build": "build-cmd",
					},
				},
				"b": {
					Scripts: map[string]string{
						"build": "build-cmd",
						"test":  "test-cmd",
					},
				},
				"c": {
					Scripts: map[string]string{
						"test": "test-cmd",
					},
				},
			})
			topoDeps := make(util.Set)
			topoDeps.Add("build")
			deps := make(util.Set)
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

			err := p.Prepare(&SchedulerExecutionOptions{
				Packages:  tc.Packages,
				TaskNames: tc.Tasks,
				TasksOnly: false,
			})

			if err != nil {
				t.Fatalf("%v", err)
			}

			errs := p.Execute(testVisitor, ExecOpts{
				Concurrency: 10,
			})

			for _, err := range errs {
				t.Fatalf("%v", err)
			}

			actual := strings.TrimSpace(p.TaskGraph.String())
			//expected := strings.TrimSpace(leafStringAll)
			expected := strings.TrimSpace(tc.Expected)
			if actual != expected {
				t.Fatalf("bad: \n\nactual---\n%s\n\n expected---\n%s", actual, expected)
			}
		})
	}
}

func TestUnknownDependency(t *testing.T) {
	g := &dag.AcyclicGraph{}
	g.Add("a")
	g.Add("b")
	g.Add("c")
	p := NewScheduler(g, map[interface{}]*fs.PackageJSON{
		"a": {
			Scripts: map[string]string{
				"build":   "build-cmd",
				"test":    "test-cmd",
				"prepare": "prepare-cmd",
			},
		},
		"b": {
			Scripts: map[string]string{
				"build":   "build-cmd",
				"test":    "test-cmd",
				"prepare": "prepare-cmd",
			},
		},
		"c": {
			Scripts: map[string]string{
				"build":   "build-cmd",
				"test":    "test-cmd",
				"prepare": "prepare-cmd",
			},
		},
	})
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

	p := NewScheduler(graph, map[interface{}]*fs.PackageJSON{
		"app1": {
			Scripts: map[string]string{
				"build": "build-cmd",
				"test":  "test-cmd",
			},
		},
		"app2": {
			Scripts: map[string]string{
				"build": "build-cmd",
				"test":  "test-cmd",
			},
		},
		"libA": {
			Scripts: map[string]string{
				"build": "build-cmd",
				"test":  "test-cmd",
			},
		},
		"libB": {
			Scripts: map[string]string{
				"build": "build-cmd",
				"test":  "test-cmd",
			},
		},
		"libC": {
			Scripts: map[string]string{
				"build": "build-cmd",
				"test":  "test-cmd",
			},
		},
		"libD": {
			Scripts: map[string]string{
				"build": "build-cmd",
				"test":  "test-cmd",
			},
		},
	})
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
	err := p.Prepare(&SchedulerExecutionOptions{
		Packages:  []string{"app2"},
		TaskNames: []string{"test"},
	})
	if err != nil {
		t.Fatalf("failed to prepare scheduler: %v", err)
	}
	errs := p.Execute(testVisitor, ExecOpts{
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

func TestSchedulerTasksOnly(t *testing.T) {
	var g dag.AcyclicGraph
	g.Add("a")
	g.Add("b")
	g.Add("c")
	g.Connect(dag.BasicEdge("c", "b"))
	g.Connect(dag.BasicEdge("c", "a"))

	p := NewScheduler(&g, map[interface{}]*fs.PackageJSON{
		"a": {
			Scripts: map[string]string{
				"build":   "build-cmd",
				"test":    "test-cmd",
				"prepare": "prepare-cmd",
			},
		},
		"b": {
			Scripts: map[string]string{
				"build":   "build-cmd",
				"test":    "test-cmd",
				"prepare": "prepare-cmd",
			},
		},
		"c": {
			Scripts: map[string]string{
				"build":   "build-cmd",
				"test":    "test-cmd",
				"prepare": "prepare-cmd",
			},
		},
	})
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

	err := p.Prepare(&SchedulerExecutionOptions{
		Packages:  []string{"a", "b", "c"},
		TaskNames: []string{"test"},
		TasksOnly: true,
	})

	if err != nil {
		t.Fatalf("%v", err)
	}

	errs := p.Execute(testVisitor, ExecOpts{
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
