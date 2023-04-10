package runsummary

import (
	"github.com/vercel/turbo/cli/internal/ci"
)

type vercelRunResponse struct {
	ID string
}

type vercelRunPayload struct {
	// ID is set by the backend, including it here for completeness, but we never fill this in.
	ID string `json:"vercelId,omitempty"`

	// StartTime is when this run was started
	StartTime int64 `json:"startTime,omitempty"`

	// EndTime is when this run ended. We will never be submitting start and endtime at the same time.
	EndTime int64 `json:"endTime,omitempty"`

	// Status is
	Status string `json:"status,omitempty"`

	// Type should be hardcoded to TURBO
	Type string `json:"type,omitempty"`

	// ExitCode is the exit code for the full run
	ExitCode int `json:"exitCode,omitempty"`

	// The command that kicked off the turbo run
	Command string `json:"command,omitempty"`

	// RepositoryPath is the relative directory from the turborepo root to where
	// the command was invoked.
	RepositoryPath string `json:"repositoryPath,omitempty"`

	// Context is the host on which this Run was executed (e.g. Vercel)
	Context string `json:"context,omitempty"`

	// TODO: we need to add these in
	// originationUser string
	// gitBranch       string
	// gitSha          string
}

type vercelCacheStatus struct {
	Status string `json:"status,omitempty"`
	Source string `json:"source,omitempty"`
}

type vercelTask struct {
	Key          string            `json:"key,omitempty"`
	Name         string            `json:"name,omitempty"`
	Workspace    string            `json:"workspace,omitempty"`
	Hash         string            `json:"hash,omitempty"`
	StartTime    int64             `json:"startTime,omitempty"`
	EndTime      int64             `json:"endTime,omitempty"`
	Cache        vercelCacheStatus `json:"cache,omitempty"`
	ExitCode     int               `json:"exitCode,omitempty"`
	Dependencies []string          `json:"dependencies,omitempty"`
	Dependents   []string          `json:"dependents,omitempty"`
}

func (rsm *Meta) newVercelRunCreatePayload() *vercelRunPayload {
	startTime := rsm.RunSummary.ExecutionSummary.startedAt.UnixMilli()
	context := "LOCAL"
	if name := ci.Constant(); name != "" {
		context = name
	}
	return &vercelRunPayload{
		StartTime:      startTime,
		Status:         "running",
		Command:        rsm.synthesizedCommand,
		RepositoryPath: rsm.repoPath.ToString(),
		Type:           "TURBO",
		Context:        context,
	}
}

func newVercelDonePayload(runsummary *RunSummary) *vercelRunPayload {
	endTime := runsummary.ExecutionSummary.endedAt.UnixMilli()
	return &vercelRunPayload{
		Status:   "completed",
		EndTime:  endTime,
		ExitCode: runsummary.ExecutionSummary.exitCode,
	}
}

func newVercelTaskPayload(taskSummary *TaskSummary) *vercelTask {
	// Set the cache source. Local and Remote shouldn't _both_ be true.
	var source string
	if taskSummary.CacheState.Local {
		source = "LOCAL"
	} else if taskSummary.CacheState.Remote {
		source = "REMOTE"
	}
	status := "MISS"
	if source != "" {
		status = "HIT"
	}

	startTime := taskSummary.Execution.startAt.UnixMilli()
	endTime := taskSummary.Execution.endTime().UnixMilli()

	return &vercelTask{
		Key:       taskSummary.TaskID,
		Name:      taskSummary.Task,
		Workspace: taskSummary.Package,
		Hash:      taskSummary.Hash,
		StartTime: startTime,
		EndTime:   endTime,
		Cache: vercelCacheStatus{
			Status: status,
			Source: source,
		},
		ExitCode:     *taskSummary.Execution.exitCode,
		Dependencies: taskSummary.Dependencies,
		Dependents:   taskSummary.Dependents,
	}
}
