package run

import (
	"testing"

	"github.com/pyr-sh/dag"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/graph"
	"github.com/vercel/turbo/cli/internal/util"
)

func Test_dontSquashTasks(t *testing.T) {
	topoGraph := &dag.AcyclicGraph{}
	topoGraph.Add("a")
	topoGraph.Add("b")
	// no dependencies between packages

	pipeline := map[string]fs.TaskDefinition{
		"build": {
			Outputs:          fs.TaskOutputs{},
			TaskDependencies: []string{"generate"},
		},
		"generate": {
			Outputs: fs.TaskOutputs{Inclusions: []string{}, Exclusions: []string{}},
		},
		"b#build": {
			Outputs: fs.TaskOutputs{Inclusions: []string{}, Exclusions: []string{}},
		},
	}
	filteredPkgs := make(util.Set)
	filteredPkgs.Add("a")
	filteredPkgs.Add("b")
	rs := &runSpec{
		FilteredPkgs: filteredPkgs,
		Targets:      []string{"build"},
		Opts:         &Opts{},
	}

	workspaceInfos := make(graph.WorkspaceInfos)
	workspaceInfos["a"] = &fs.PackageJSON{
		Name:    "a",
		Scripts: map[string]string{},
	}

	completeGraph := &graph.CompleteGraph{
		WorkspaceGraph: *topoGraph,
		Pipeline:       pipeline,
		WorkspaceInfos: workspaceInfos,
	}

	engine, err := buildTaskGraphEngine(completeGraph, rs)
	if err != nil {
		t.Fatalf("failed to build task graph: %v", err)
	}
	toRun := engine.TaskGraph.Vertices()
	// 4 is the 3 tasks + root
	if len(toRun) != 4 {
		t.Errorf("expected 4 tasks, got %v", len(toRun))
	}
	for task := range pipeline {
		if _, ok := engine.Tasks[task]; !ok {
			t.Errorf("expected to find task %v in the task graph, but it is missing", task)
		}
	}
}

func Test_taskSelfRef(t *testing.T) {
	topoGraph := &dag.AcyclicGraph{}
	topoGraph.Add("a")
	// no dependencies between packages

	pipeline := map[string]fs.TaskDefinition{
		"build": {
			TaskDependencies: []string{"build"},
		},
	}
	filteredPkgs := make(util.Set)
	filteredPkgs.Add("a")
	rs := &runSpec{
		FilteredPkgs: filteredPkgs,
		Targets:      []string{"build"},
		Opts:         &Opts{},
	}

	completeGraph := &graph.CompleteGraph{
		WorkspaceGraph: *topoGraph,
		Pipeline:       pipeline,
	}

	_, err := buildTaskGraphEngine(completeGraph, rs)
	if err == nil {
		t.Fatalf("expected to failed to build task graph: %v", err)
	}
}
