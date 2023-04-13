package runsummary

import (
	"github.com/vercel/turbo/cli/internal/ci"
)

// spacesRunResponse deserialized the response from POST Run endpoint
type spacesRunResponse struct {
	ID  string
	URL string
}

type spacesRunPayload struct {
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

	// Context is the host on which this Run was executed (e.g. Github Action, Vercel, etc)
	Context string `json:"context,omitempty"`

	// TODO: we need to add these in
	// originationUser string
	// gitBranch       string
	// gitSha          string
}

// spacesCacheStatus is the same as TaskCacheSummary so we can convert
// spacesCacheStatus(cacheSummary), but change the json tags, to omit local and remote fields
type spacesCacheStatus struct {
	// omitted fields, but here so we can convert from TaskCacheSummary easily
	Local     bool   `json:"-"`
	Remote    bool   `json:"-"`
	Status    string `json:"status"` // should always be there
	Source    string `json:"source,omitempty"`
	TimeSaved int    `json:"timeSaved"`
}

type spacesTask struct {
	Key          string            `json:"key,omitempty"`
	Name         string            `json:"name,omitempty"`
	Workspace    string            `json:"workspace,omitempty"`
	Hash         string            `json:"hash,omitempty"`
	StartTime    int64             `json:"startTime,omitempty"`
	EndTime      int64             `json:"endTime,omitempty"`
	Cache        spacesCacheStatus `json:"cache,omitempty"`
	ExitCode     int               `json:"exitCode,omitempty"`
	Dependencies []string          `json:"dependencies,omitempty"`
	Dependents   []string          `json:"dependents,omitempty"`
}

func (rsm *Meta) newSpacesRunCreatePayload() *spacesRunPayload {
	startTime := rsm.RunSummary.ExecutionSummary.startedAt.UnixMilli()
	context := "LOCAL"
	if name := ci.Constant(); name != "" {
		context = name
	}
	return &spacesRunPayload{
		StartTime:      startTime,
		Status:         "running",
		Command:        rsm.synthesizedCommand,
		RepositoryPath: rsm.repoPath.ToString(),
		Type:           "TURBO",
		Context:        context,
	}
}

func newSpacesDonePayload(runsummary *RunSummary) *spacesRunPayload {
	endTime := runsummary.ExecutionSummary.endedAt.UnixMilli()
	return &spacesRunPayload{
		Status:   "completed",
		EndTime:  endTime,
		ExitCode: runsummary.ExecutionSummary.exitCode,
	}
}

func newSpacesTaskPayload(taskSummary *TaskSummary) *spacesTask {
	startTime := taskSummary.Execution.startAt.UnixMilli()
	endTime := taskSummary.Execution.endTime().UnixMilli()

	return &spacesTask{
		Key:          taskSummary.TaskID,
		Name:         taskSummary.Task,
		Workspace:    taskSummary.Package,
		Hash:         taskSummary.Hash,
		StartTime:    startTime,
		EndTime:      endTime,
		Cache:        spacesCacheStatus(taskSummary.CacheSummary), // wrapped so we can remove fields
		ExitCode:     *taskSummary.Execution.exitCode,
		Dependencies: taskSummary.Dependencies,
		Dependents:   taskSummary.Dependents,
	}
}
