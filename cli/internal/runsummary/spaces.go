package runsummary

import (
	"encoding/json"
	"fmt"
	"sync"

	"github.com/mitchellh/cli"
	"github.com/vercel/turbo/cli/internal/ci"
	"github.com/vercel/turbo/cli/internal/client"
)

const runsEndpoint = "/v0/spaces/%s/runs"
const runsPatchEndpoint = "/v0/spaces/%s/runs/%s"
const tasksEndpoint = "/v0/spaces/%s/runs/%s/tasks"

// spaceRequest contains all the information for a single request to Spaces
type spaceRequest struct {
	method  string
	url     string
	makeURL func(self *spaceRequest, r *spaceRun) error // Should set url on self
	body    interface{}
	onDone  func(req *spaceRequest, response []byte)
}

func (req *spaceRequest) debug(msg string) {
	fmt.Printf("[%s] %s: %s\n", req.method, req.url, msg)
}

func (req *spaceRequest) error(msg string) error {
	return fmt.Errorf("[%s] %s: %s", req.method, req.url, msg)
}

type spacesClient struct {
	requests chan *spaceRequest
	errors   []error
	api      *client.APIClient
	ui       cli.Ui
	run      *spaceRun
	wg       sync.WaitGroup
}

type spaceRun struct {
	ID  string
	URL string
}

func newSpacesClient(api *client.APIClient, ui cli.Ui) *spacesClient {
	c := &spacesClient{
		api:      api,
		ui:       ui,
		requests: make(chan *spaceRequest), // TODO: give this a size based on tasks
	}

	// Start receiving requests
	// TODO: how to make this goroutine block on the very first request?
	go func() {
		for req := range c.requests {
			c.makeRequest(req)
		}
	}()

	return c
}

func (c *spacesClient) asyncRequest(req *spaceRequest) {
	c.wg.Add(1) // increment waitgroup counter
	req.debug("Queuing")
	c.requests <- req
}

func (c *spacesClient) makeRequest(req *spaceRequest) {
	defer c.wg.Done() // decrement waitgroup counter

	if !c.api.IsLinked() {
		c.errors = append(c.errors, req.error("Repo is not linked to a Space. Run `turbo link --target=spaces` first"))
		return
	}

	// We only care about POST and PATCH right now
	if req.method != "POST" && req.method != "PATCH" {
		c.errors = append(c.errors, req.error(fmt.Sprintf("Unsupported method %s", req.method)))
		return
	}

	payload, err := json.Marshal(req.body)
	if err != nil {
		c.errors = append(c.errors, req.error(fmt.Sprintf("Failed to create payload: %s", err)))
		return
	}

	// Make the request
	var resp []byte
	var reqErr error

	// Lazily make the url with the run object since we need the run.ID
	if req.makeURL != nil {
		if err := req.makeURL(req, c.run); err != nil {
			c.errors = append(c.errors, err)
			return
		}
	}

	req.debug("Executing")
	if req.method == "POST" {
		resp, reqErr = c.api.JSONPost(req.url, payload)
	} else if req.method == "PATCH" {
		resp, reqErr = c.api.JSONPatch(req.url, payload)
	} else {
		c.errors = append(c.errors, req.error("Unsupported request method"))
	}

	if reqErr != nil {
		c.errors = append(c.errors, req.error(fmt.Sprintf("%s", reqErr)))
		return
	}

	// Call the onDone handler if there is one
	if req.onDone != nil {
		req.onDone(req, resp)
	}
}

func (c *spacesClient) start(rsm *Meta) {
	if rsm.spaceID == "" {
		c.errors = append(c.errors, fmt.Errorf("No spaceID found to post run"))
		return
	}

	// Set a default, empty one here, so we'll have something downstream and not a segfault
	c.run = &spaceRun{}

	c.asyncRequest(&spaceRequest{
		method: "POST",
		url:    fmt.Sprintf(runsEndpoint, rsm.spaceID),
		body:   newSpacesRunCreatePayload(rsm),

		// handler for when the request finishes. We set the response into a struct on the client
		// because we need the run ID and URL from the server later.
		onDone: func(req *spaceRequest, response []byte) {
			if response == nil {
				return
			}

			if err := json.Unmarshal(response, c.run); err != nil {
				c.errors = append(c.errors, fmt.Errorf("Error unmarshaling response: %w", err))
			}
		},
	})
}

func (c *spacesClient) postTask(rsm *Meta, task *TaskSummary) {
	if rsm.spaceID == "" {
		c.errors = append(c.errors, fmt.Errorf("No spaceID found to post %s", task.TaskID))
		return
	}

	if c.run.ID == "" {
		c.errors = append(c.errors, fmt.Errorf("No Run ID found to post task %s", task.TaskID))
		return
	}

	c.asyncRequest(&spaceRequest{
		method: "POST",
		makeURL: func(self *spaceRequest, run *spaceRun) error {
			if run.ID == "" {
				return fmt.Errorf("No Run ID found to send PATCH request")
			}
			self.url = fmt.Sprintf(tasksEndpoint, rsm.spaceID, run.ID)
			return nil
		},
		body: newSpacesTaskPayload(task),
	})
}

func (c *spacesClient) done(rsm *Meta) {
	if rsm.spaceID == "" {
		c.errors = append(c.errors, fmt.Errorf("No spaceID found to send PATCH request"))
		return
	}

	c.asyncRequest(&spaceRequest{
		method: "PATCH",
		makeURL: func(self *spaceRequest, run *spaceRun) error {
			if run.ID == "" {
				return fmt.Errorf("No Run ID found to send PATCH request")
			}
			self.url = fmt.Sprintf(runsPatchEndpoint, rsm.spaceID, run.ID)
			return nil
		},
		body: newSpacesDonePayload(rsm.RunSummary),
	})

	// Close the channel since we are now down with all requests
	close(c.requests)
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
