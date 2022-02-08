package core

import (
	"fmt"
	"strings"
	"testing"
	"github.com/vercel/turborepo/cli/internal/util"

	"github.com/pyr-sh/dag"
)

func TestSchedulerDefault(t *testing.T) {
	var g dag.AcyclicGraph
	g.Add("a")
	g.Add("b")
	g.Add("c")
	g.Connect(dag.BasicEdge("c", "b"))
	g.Connect(dag.BasicEdge("c", "a"))

	p := NewScheduler(&g)
	topoDeps := make(util.Set)
	topoDeps.Add("build")
	deps := make(util.Set)
	deps.Add("prepare")
	p.AddTask(&Task{
		Name:     "build",
		TopoDeps: topoDeps,
		Deps:     deps,
		Run: func(cwd string) error {
			fmt.Println(cwd)
			return nil
		},
	})
	p.AddTask(&Task{
		Name:     "test",
		TopoDeps: topoDeps,
		Deps:     deps,
		Run: func(cwd string) error {
			fmt.Println(cwd)
			return nil
		},
	})
	p.AddTask(&Task{
		Name: "prepare",
		Run: func(cwd string) error {
			fmt.Println(cwd)
			return nil
		},
	})
	p.AddTask(&Task{
		Name: "side-quest", // not in the build/test tree
		Deps: deps,
		Run: func(cwd string) error {
			fmt.Println(cwd)
			return nil
		},
	})

	if _, ok := p.Tasks["build"]; !ok {
		t.Fatal("AddTask is not adding tasks (build)")
	}

	if _, ok := p.Tasks["test"]; !ok {
		t.Fatal("AddTask is not adding tasks (test)")
	}

	err := p.Prepare(&SchedulerExecutionOptions{
		Packages:    nil,
		TaskNames:   []string{"test"},
		Concurrency: 10,
		Parallel:    false,
		TasksOnly:   false,
	})

	if err != nil {
		t.Fatalf("%v", err)
	}

	errs := p.Execute()

	for _, err := range errs {
		t.Fatalf("%v", err)
	}

	actual := strings.TrimSpace(p.TaskGraph.String())
	expected := strings.TrimSpace(leafStringAll)
	if actual != expected {
		t.Fatalf("bad: \n\nactual---\n%s\n\n expected---\n%s", actual, expected)
	}
}

func TestSchedulerTasksOnly(t *testing.T) {
	var g dag.AcyclicGraph
	g.Add("a")
	g.Add("b")
	g.Add("c")
	g.Connect(dag.BasicEdge("c", "b"))
	g.Connect(dag.BasicEdge("c", "a"))

	p := NewScheduler(&g)
	topoDeps := make(util.Set)
	topoDeps.Add("build")
	deps := make(util.Set)
	deps.Add("prepare")
	p.AddTask(&Task{
		Name:     "build",
		TopoDeps: topoDeps,
		Deps:     deps,
		Run: func(cwd string) error {
			fmt.Println(cwd)
			return nil
		},
	})
	p.AddTask(&Task{
		Name:     "test",
		TopoDeps: topoDeps,
		Deps:     deps,
		Run: func(cwd string) error {
			fmt.Println(cwd)
			return nil
		},
	})
	p.AddTask(&Task{
		Name: "prepare",
		Run: func(cwd string) error {
			fmt.Println(cwd)
			return nil
		},
	})

	if _, ok := p.Tasks["build"]; !ok {
		t.Fatal("AddTask is not adding tasks (build)")
	}

	if _, ok := p.Tasks["test"]; !ok {
		t.Fatal("AddTask is not adding tasks (test)")
	}

	err := p.Prepare(&SchedulerExecutionOptions{
		Packages:    nil,
		TaskNames:   []string{"test"},
		Concurrency: 10,
		Parallel:    false,
		TasksOnly:   true,
	})

	if err != nil {
		t.Fatalf("%v", err)
	}

	errs := p.Execute()

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
