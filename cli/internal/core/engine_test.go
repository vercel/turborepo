package core

import (
	"errors"
	"testing"

	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/graph"
	"github.com/vercel/turbo/cli/internal/workspace"
	"gotest.tools/v3/assert"

	"github.com/pyr-sh/dag"
)

func TestShortCircuiting(t *testing.T) {
	var workspaceGraph dag.AcyclicGraph
	workspaceGraph.Add("a")
	workspaceGraph.Add("b")
	workspaceGraph.Add("c")
	// Dependencies: a -> b -> c
	workspaceGraph.Connect(dag.BasicEdge("a", "b"))
	workspaceGraph.Connect(dag.BasicEdge("b", "c"))

	buildTask := &fs.BookkeepingTaskDefinition{}
	err := buildTask.UnmarshalJSON([]byte("{\"dependsOn\": [\"^build\"]}"))
	assert.NilError(t, err, "BookkeepingTaskDefinition unmarshall")

	pipeline := map[string]fs.BookkeepingTaskDefinition{
		"build": *buildTask,
	}

	p := NewEngine(&graph.CompleteGraph{
		WorkspaceGraph:  workspaceGraph,
		Pipeline:        pipeline,
		TaskDefinitions: map[string]*fs.TaskDefinition{},
		WorkspaceInfos: workspace.Catalog{
			PackageJSONs: map[string]*fs.PackageJSON{
				"//": {},
				"a":  {},
				"b":  {},
				"c":  {},
			},
			TurboConfigs: map[string]*fs.TurboJSON{
				"//": {
					Pipeline: pipeline,
				},
			},
		},
	}, false)

	p.AddTask("build")

	err = p.Prepare(&EngineBuildingOptions{
		Packages:  []string{"a", "b", "c"},
		TaskNames: []string{"build"},
		TasksOnly: false,
	})

	if err != nil {
		t.Fatalf("%v", err)
	}

	executed := map[string]bool{
		"a#build": false,
		"b#build": false,
		"c#build": false,
	}
	expectedErr := errors.New("an error occurred")
	// b#build is going to error, we expect to not execute a#build, which depends on b
	testVisitor := func(taskID string) error {
		println(taskID)
		executed[taskID] = true
		if taskID == "b#build" {
			return expectedErr
		}
		return nil
	}

	errs := p.Execute(testVisitor, EngineExecutionOptions{
		Concurrency: 10,
	})
	assert.Equal(t, len(errs), 1)
	assert.Equal(t, errs[0], expectedErr)

	assert.Equal(t, executed["c#build"], true)
	assert.Equal(t, executed["b#build"], true)
	assert.Equal(t, executed["a#build"], false)
}
