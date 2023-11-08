package runsummary

import (
	"context"
	"encoding/json"
	"fmt"
	"sync"
	"time"

	"github.com/mitchellh/cli"
	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/ci"
)

const runsEndpoint = "/v0/spaces/%s/runs"
const runsPatchEndpoint = "/v0/spaces/%s/runs/%s"
const tasksEndpoint = "/v0/spaces/%s/runs/%s/tasks"

// spaceRequest contains all the information for a single request to Spaces
// This will be an enum in Rust with all the relevant information to construct the url.
// We'll pattern match and call the correct API client method.
type spaceRequest struct {
	method  string
	url     string
	body    interface{}
	makeURL func(self *spaceRequest, r *spaceRun) error // Should set url on self
	//onDone  func(self *spaceRequest, response []byte, err error) // Handler for when request completes
}

type spacesAPIClient interface {
	JSONPost(ctx context.Context, url string, payload []byte) ([]byte, error)
	JSONPatch(ctx context.Context, url string, payload []byte) ([]byte, error)
	IsLinked() bool
}

type spacesClient struct {
	requests       chan *spaceRequest
	api            spacesAPIClient
	run            *spaceRun
	runCreated     chan struct{}
	runCreateError error
	wg             sync.WaitGroup
	spaceID        string
	enabled        bool
	requestTimeout time.Duration

	errMu  sync.Mutex
	errors []error
}

type spaceRun struct {
	ID  string
	URL string
}

func newSpacesClient(spaceID string, api spacesAPIClient) *spacesClient {
	c := &spacesClient{
		api:            api,
		spaceID:        spaceID,
		enabled:        false,                    // Start with disabled
		requests:       make(chan *spaceRequest), // TODO: give this a size based on tasks
		runCreated:     make(chan struct{}, 1),
		run:            &spaceRun{},
		requestTimeout: 10 * time.Second,
	}

	if spaceID == "" {
		return c
	}

	if !c.api.IsLinked() {
		c.errors = append(c.errors, fmt.Errorf("Error: experimentalSpaceId is enabled, but repo is not linked to API. Run `turbo link` or `turbo login` first"))
		return c
	}

	// Explicitly enable if all conditions are met
	c.enabled = true

	return c
}

// Start receiving and processing requests in 8 goroutines
// There is an additional marker (protected by a mutex) that indicates
// when the first request is done. All other requests are blocked on that one.
// This first request is the POST /run request. We need to block on it because
// the response contains the run ID from the server, which we need to construct the
// URLs of subsequent requests.
func (c *spacesClient) start() {
	// Start an immediately invoked go routine that listens for requests coming in from a channel
	pending := []*spaceRequest{}

	// Create a labeled statement so we can break out of the for loop more easily

	// Setup a for loop that goes infinitely until we break out of it
FirstRequest:
	for {
		// A select statement that can listen for messages from multiple channels
		select {
		// listen for new requests coming in
		case req, isOpen := <-c.requests:
			// If we read from the channel and its already closed, it means
			// something went wrong and we are done with the run, but the first
			// request either never happened or didn't write to the c.runCreated channel
			// to signal that its done. In this case, we need to break out of the forever loop.
			if !isOpen {
				break FirstRequest
			}
			// Queue everything. When the first request is done,
			// we'll get a message on the other channel and break out of this loop
			pending = append(pending, req)
			// Wait for c.runCreated channel to be closed and:
		case <-c.runCreated:
			// 1. flush pending requests
			for _, req := range pending {
				go c.dequeueRequest(req)
			}

			// 2. break out of the forever loop.
			break FirstRequest
		}
	}

	// and then continue listening for more requests as they come in until the channel is closed
	for req := range c.requests {
		go c.dequeueRequest(req)
	}
}

func makeRequest(ctx context.Context, api spacesAPIClient, req *spaceRequest, run *spaceRun) ([]byte, error) {
	// The runID is required for POST task requests and PATCH run request URLS,
	// so we have to construct these URLs lazily with a `makeURL` affordance.
	//
	// We are assuming that if makeURL is defined, this is NOT the first request.
	// This is not a great assumption, and will fail if our endpoint URLs change later.
	//
	// Secondly, if makeURL _is_ defined, we call it, and if there are any errors, we exit early.
	// We are doing this check before any of the other basic checks (e.g. the existence of a spaceID)
	// because in the case the repo is not linked to a space, we don't want to print those errors
	// for every request that fails. On the other hand, if that POST /run request fails, and N
	// requests fail after that as a consequence, it is ok to print all of those errors.
	//
	// We're going to remove this in the Rust version because closures are kinda messy, especially closures that mutate.
	// Instead we'll directly store the id in the struct and use that.
	if req.makeURL != nil {
		if err := req.makeURL(req, run); err != nil {
			return nil, err
		}
	}

	payload, err := json.Marshal(req.body)
	if err != nil {
		return nil, err
	}

	// Make the request
	if req.method == "POST" {
		return api.JSONPost(ctx, req.url, payload)
	} else if req.method == "PATCH" {
		return api.JSONPatch(ctx, req.url, payload)
	}
	panic(fmt.Sprintf("Unsupported method %v", req.method))
}

func (c *spacesClient) createRun(payload *spacesRunPayload) {
	c.wg.Add(1)
	// We don't need the goroutine here since tokio will schedule the threads for us.
	// If needed we can do `spawn`
	go func() {
		defer c.wg.Done()
		defer close(c.runCreated)
		ctx, cancel := context.WithTimeout(context.Background(), c.requestTimeout)
		defer cancel()

		// Because `makeUrl` is not defined, this is the first request.
		req := &spaceRequest{
			method: "POST",
			url:    fmt.Sprintf(runsEndpoint, c.spaceID),
			body:   payload,
		}
		// In the Rust version we'll have a specific `create_spaces_run` method
		resp, err := makeRequest(ctx, c.api, req, c.run)
		// We don't need to store this error in the Rust version. Instead
		// if this fails, we send a message via a oneshot channel to indicate
		// that we don't need to send any more requests.
		if err != nil {
			c.runCreateError = err
			return
		}
		if err := json.Unmarshal(resp, c.run); err != nil {
			c.runCreateError = errors.Wrap(err, "failed to unmarshal create run response")
		}
	}()
}

func (c *spacesClient) postTask(task *TaskSummary, logs []byte) {
	c.queueRequest(&spaceRequest{
		method: "POST",
		makeURL: func(self *spaceRequest, run *spaceRun) error {
			if run.ID == "" {
				return fmt.Errorf("No Run ID found to post task %s", task.TaskID)
			}
			self.url = fmt.Sprintf(tasksEndpoint, c.spaceID, run.ID)
			return nil
		},
		body: newSpacesTaskPayload(task, logs),
	})
}

func (c *spacesClient) finishRun(rsm *Meta) {
	c.queueRequest(&spaceRequest{
		method: "PATCH",
		makeURL: func(self *spaceRequest, run *spaceRun) error {
			if run.ID == "" {
				return fmt.Errorf("No Run ID found to send PATCH request")
			}
			self.url = fmt.Sprintf(runsPatchEndpoint, c.spaceID, run.ID)
			return nil
		},
		body: newSpacesDonePayload(rsm.RunSummary),
	})
}

// queueRequest adds the given request to the requests channel and increments the waitGroup counter
func (c *spacesClient) queueRequest(req *spaceRequest) {
	c.wg.Add(1)
	c.requests <- req
}

// dequeueRequest makes the request in a go routine and decrements the waitGroup counter
func (c *spacesClient) dequeueRequest(req *spaceRequest) {
	defer c.wg.Done()
	// Only send requests if we successfully created the Run
	if c.runCreateError != nil {
		return
	}
	ctx, cancel := context.WithTimeout(context.Background(), c.requestTimeout)
	defer cancel()
	_, err := makeRequest(ctx, c.api, req, c.run)
	if err != nil {
		c.errMu.Lock()
		defer c.errMu.Unlock()
		c.errors = append(c.errors, err)
	}
}

func (c *spacesClient) printErrors(ui cli.Ui) {
	// Print any errors
	if len(c.errors) > 0 {
		for _, err := range c.errors {
			ui.Warn(fmt.Sprintf("%s", err))
		}
	}
}

// Close will wait for all requests to finish and then close the channel listening for them
func (c *spacesClient) Close() {
	// wait for all requests to finish.
	c.wg.Wait()

	// close out the channel, since there should be no more requests.
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
	ExitCode       *int                `json:"exitCode,omitempty"`       // exit code for the full run
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
	ExitCode     *int              `json:"exitCode,omitempty"`
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
		ExitCode: &runsummary.ExecutionSummary.exitCode,
	}
}

func newSpacesTaskPayload(taskSummary *TaskSummary, logs []byte) *spacesTask {
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
		ExitCode:     taskSummary.Execution.exitCode,
		Dependencies: taskSummary.Dependencies,
		Dependents:   taskSummary.Dependents,
		Logs:         string(logs),
	}
}
