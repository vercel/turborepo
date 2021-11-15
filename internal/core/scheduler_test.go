package core

import (
	"fmt"
	"strings"
	"testing"
	"turbo/internal/util"

	"github.com/pyr-sh/dag"
)

func TestSchedulerAddTask(t *testing.T) {
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
	})

	if err != nil {
		t.Fatalf("%v", err)
	}

	errs := p.Execute()

	for _, err := range errs {
		t.Fatalf("%v", err)
	}

	actual := strings.TrimSpace(p.TaskGraph.String())
	expected := strings.TrimSpace(leafString)
	if actual != expected {
		t.Fatalf("bad: \n\nactual---\n%s\n\n expected---\n%s", actual, expected)
	}
}

const leafString = `
___ROOT___
a#build
  ___ROOT___
a#test
  ___ROOT___
b#build
  ___ROOT___
b#test
  ___ROOT___
c#test
  a#build
  b#build
`
