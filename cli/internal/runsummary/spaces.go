package runsummary

import (
	"github.com/vercel/turbo/cli/internal/ci"
)

// spacesRunResponse deserialized the response from POST Run endpoint
type spacesRunResponse struct {
	ID  string
	URL string
}

type spacesClientSummary struct {
	ID      string `json:"id"`
	Name    string `json:"name"`
	Version string `json:"version"`
}

type spacesRunPayload struct {
	StartTime      int64               `json:"startTime,omitempty"`      // when the run was started
	EndTime        int64               `json:"endTime,omitempty"`        // when the run ended. we should never submit start and end at the same time.
	Status         string              `json:"status,omitempty"`         // Status is "running" or "completed"
	Type           string              `json:"type,omitempty"`           // hardcoded to "TURBO"
	ExitCode       int                 `json:"exitCode,omitempty"`       // exit code for the full run
	Command        string              `json:"command,omitempty"`        // the thing that kicked off the turbo run
	RepositoryPath string              `json:"repositoryPath,omitempty"` // where the command was invoked from
	Context        string              `json:"context,omitempty"`        // the host on which this Run was executed (e.g. Github Action, Vercel, etc)
	Client         spacesClientSummary `json:"client"`                   // Details about the turbo client
	GitBranch      string              `json:"gitBranch"`
	GitSha         string              `json:"gitSha"`

	// TODO: we need to add these in
	// originationUser string
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
	Logs         string            `json:"log"`
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
		GitBranch:      rsm.RunSummary.SCM.Branch,
		GitSha:         rsm.RunSummary.SCM.Sha,
		Client: spacesClientSummary{
			ID:      "turbo",
			Name:    "Turbo",
			Version: rsm.RunSummary.TurboVersion,
		},
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
		Logs:         string(taskSummary.GetLogs()),
	}
}
