package runsummary

import (
	"fmt"
	"strings"

	"github.com/vercel/turbo/cli/internal/ci"
	"github.com/vercel/turbo/cli/internal/util"
)

type vercelRunResponse struct {
	ID string
}

type vercelRunPayload struct {
	// ID is set by the backend, including it here for completeness, but we never fill this in.
	ID string `json:"vercelId,omitempty"`

	// StartTime is when this run was started
	StartTime int `json:"startTime,omitempty"`

	// EndTime is when this run ended. We will never be submitting start and endtime at the same time.
	EndTime int `json:"endTime,omitempty"`

	// Status is
	Status string `json:"status,omitempty"`

	// Type should be hardcoded to TURBO
	Type string `json:"type,omitempty"`

	// ExitCode is the exit code for the full run
	ExitCode int `json:"exitCode,omitempty"`

	// The command that kicked off the turbo run
	Command string `json:"command,omitempty"`

	Context string `json:"context,omitempy"`

	// TODO: we need to add these in
	// originationUser string
	// gitBranch       string
	// gitSha          string
	// command         string
}

type vercelCacheStatus struct {
	Status string `json:"status,omitempty"`
	Source string `json:"source,omitempty"`
}

type vercelTask struct {
	// id  string
	// log string
	// TODO: add in command

	Key          string            `json:"key,omitempty"`
	Name         string            `json:"name,omitempty"`
	Workspace    string            `json:"workspace,omitempty"`
	Hash         string            `json:"hash,omitempty"`
	StartTime    int               `json:"startTime,omitempty"`
	EndTime      int               `json:"endTime,omitempty"`
	Cache        vercelCacheStatus `json:"cache,omitempty"`
	ExitCode     int               `json:"exitCode,omitempty"`
	Dependencies []string          `json:"dependencies,omitempty"`
	Dependents   []string          `json:"dependents,omitempty"`
}

func newVercelRunCreatePayload(runsummary *RunSummary) *vercelRunPayload {
	startTime := runsummary.ExecutionSummary.startedAt.UnixMilli()
	taskNames := make(util.Set, len(runsummary.Tasks))
	for _, task := range runsummary.Tasks {
		taskNames.Add(task.Task)
	}
	return &vercelRunPayload{
		StartTime: int(startTime),
		Status:    "running",
		Command:   fmt.Sprintf("turbo run %s", strings.Join(taskNames.UnsafeListOfStrings(), " ")),
		Type:      "TURBO",
		Context:   getContext(),
	}
}

func getContext() string {
	name := ci.Constant()
	if name == "" {
		return "LOCAL"
	}

	return name

}

func newVercelDonePayload(runsummary *RunSummary) *vercelRunPayload {
	endTime := runsummary.ExecutionSummary.endedAt.UnixMilli()
	return &vercelRunPayload{
		Status:   "completed",
		EndTime:  int(endTime),
		ExitCode: runsummary.ExecutionSummary.exitCode,
	}
}

func newVercelTaskPayload(taskSummary *TaskSummary) *vercelTask {
	hit := taskSummary.CacheState.Local || taskSummary.CacheState.Remote
	status := "MISS"
	var source string
	if hit {
		source = "REMOTE"
		status = "HIT"
	}

	return &vercelTask{
		Key:       taskSummary.TaskID,
		Name:      taskSummary.Task,
		Workspace: taskSummary.Package,
		Hash:      taskSummary.Hash,
		StartTime: int(taskSummary.Execution.startAt.UnixMilli()),
		EndTime:   int(taskSummary.Execution.startAt.Add(taskSummary.Execution.Duration).UnixMilli()),
		Cache: vercelCacheStatus{
			Status: status,
			Source: source,
		},
		ExitCode:     *taskSummary.Execution.exitCode,
		Dependencies: taskSummary.Dependencies,
		Dependents:   taskSummary.Dependents,
	}
}
