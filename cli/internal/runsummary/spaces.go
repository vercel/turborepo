package runsummary

import (
	"encoding/json"
	"fmt"

	"github.com/mitchellh/cli"
	"github.com/vercel/turbo/cli/internal/ci"
	"github.com/vercel/turbo/cli/internal/client"
)

const runsEndpoint = "/v0/spaces/%s/runs"
const runsPatchEndpoint = "/v0/spaces/%s/runs/%s"
const tasksEndpoint = "/v0/spaces/%s/runs/%s/tasks"

// spaceRequest contains all the information for a single request to Spaces
type spaceRequest struct {
	method   string
	url      string
	body     interface{}
	response []byte
	err      error
}

type spacesClient struct {
	requests []*spaceRequest
	errors   []error
	api      *client.APIClient
	ui       cli.Ui
	run      *spaceRun
}

type spaceRun struct {
	ID  string
	URL string
}

func (c *spacesClient) makeRequest(req *spaceRequest) {
	// closure to make errors so we can consistently get the request details
	makeError := func(msg string) error {
		return fmt.Errorf("%s: %s - %s", req.method, req.url, msg)
	}

	// We only care about POST and PATCH right now
	if req.method != "POST" && req.method != "PATCH" {
		c.errors = append(c.errors, makeError(fmt.Sprintf("Unsupported method %s", req.method)))
		return
	}

	payload, err := json.Marshal(req.body)
	if err != nil {
		c.errors = append(c.errors, makeError(fmt.Sprintf("Failed to create payload: %s", err)))
		return
	}

	// Make the request
	var resp []byte
	var reqErr error
	if req.method == "POST" {
		resp, reqErr = c.api.JSONPost(req.url, payload)
	} else if req.method == "PATCH" {
		resp, reqErr = c.api.JSONPatch(req.url, payload)
	} else {
		c.errors = append(c.errors, makeError("Spaces client: Unsupported method"))
	}

	if reqErr != nil {
		req.err = makeError(fmt.Sprintf("%s", reqErr))
		return
	}

	// If there are no errors, we can assign the response back to the request so we can read it later
	req.response = resp

	// Append into global requests
	c.requests = append(c.requests, req)
}

func (c *spacesClient) start(rsm *Meta) {
	if !c.api.IsLinked() {
		c.errors = append(c.errors, fmt.Errorf("Failed to post to space because repo is not linked to a Space. Run `turbo link` first"))
		return
	}

	req := &spaceRequest{
		method: "POST",
		url:    fmt.Sprintf(runsEndpoint, rsm.spaceID),
		body:   newSpacesRunCreatePayload(rsm),
	}

	// This will assign the response to the request if all is well
	c.makeRequest(req)

	// Set a default, empty one here, so we'll have something downstream and not a segfault
	c.run = &spaceRun{}

	if req.response == nil {
		return
	}

	// unmarshal the response into our c.run struct and catch errors
	if err := json.Unmarshal(req.response, c.run); err != nil {
		c.errors = append(c.errors, fmt.Errorf("Error unmarshaling response: %w", err))
	}
}

func (c *spacesClient) postTask(rsm *Meta, task *TaskSummary) {
	if !c.api.IsLinked() {
		c.errors = append(c.errors, fmt.Errorf("Failed to post %s, because repo is not linked to a Space. Run `turbo link` first", task.TaskID))
		return
	}

	if rsm.spaceID == "" {
		c.errors = append(c.errors, fmt.Errorf("No spaceID found to post %s", task.TaskID))
		return
	}

	if c.run.ID == "" {
		c.errors = append(c.errors, fmt.Errorf("No Run ID found to post task %s", task.TaskID))
		return
	}

	c.makeRequest(&spaceRequest{
		method: "POST",
		url:    fmt.Sprintf(tasksEndpoint, rsm.spaceID, c.run.ID),
		body:   newSpacesTaskPayload(task),
	})
}

func (c *spacesClient) done(rsm *Meta) {
	if !c.api.IsLinked() {
		c.errors = append(c.errors, fmt.Errorf("Failed to post to space because repo is not linked to a Space. Run `turbo link` first"))
		return
	}

	if rsm.spaceID == "" {
		c.errors = append(c.errors, fmt.Errorf("No spaceID found to send PATCH request"))
		return
	}

	if c.run.ID == "" {
		c.errors = append(c.errors, fmt.Errorf("No Run ID found to send PATCH request"))
		return
	}

	c.makeRequest(&spaceRequest{
		method: "PATCH",
		url:    fmt.Sprintf(runsPatchEndpoint, rsm.spaceID, c.run.ID),
		body:   newSpacesDonePayload(rsm.RunSummary),
	})
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
	User           string              `json:"originationUser,omitempty"`
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

func newSpacesRunCreatePayload(rsm *Meta) *spacesRunPayload {
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
		User:           rsm.RunSummary.User,
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
