// Package runsummary implements structs that report on a `turbo run` and `turbo run --dry`
package runsummary

import (
	"encoding/json"
	"fmt"
	"path/filepath"
	"sync"
	"time"

	"github.com/mitchellh/cli"
	"github.com/segmentio/ksuid"
	"github.com/vercel/turbo/cli/internal/client"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// MissingTaskLabel is printed when a package is missing a definition for a task that is supposed to run
// E.g. if `turbo run build --dry` is run, and package-a doesn't define a `build` script in package.json,
// the RunSummary will print this, instead of the script (e.g. `next build`).
const MissingTaskLabel = "<NONEXISTENT>"

// MissingFrameworkLabel is a string to identify when a workspace doesn't detect a framework
const MissingFrameworkLabel = "<NO FRAMEWORK DETECTED>"

const runSummarySchemaVersion = "0"
const runsEndpoint = "/v0/spaces/%s/runs"
const tasksEndpoint = "/v0/spaces/%s/runs/%s/tasks"

// Meta is a wrapper around the serializable RunSummary, with some extra information
// about the Run and references to other things that we need.
type Meta struct {
	RunSummary    *RunSummary
	ui            cli.Ui
	singlePackage bool
	shouldSave    bool
	apiClient     *client.APIClient
	spaceID       string
}

// RunSummary contains a summary of what happens in the `turbo run` command and why.
type RunSummary struct {
	ID                ksuid.KSUID        `json:"id"`
	Version           string             `json:"version"`
	TurboVersion      string             `json:"turboVersion"`
	GlobalHashSummary *GlobalHashSummary `json:"globalHashSummary"`
	Packages          []string           `json:"packages"`
	ExecutionSummary  *executionSummary  `json:"executionSummary"`
	Tasks             []*TaskSummary     `json:"tasks"`
}

// singlePackageRunSummary is the same as RunSummary with some adjustments
// to the internal struct for a single package. It's likely that we can use the
// same struct for Single Package repos in the future.
type singlePackageRunSummary struct {
	Tasks []singlePackageTaskSummary `json:"tasks"`
}

// NewRunSummary returns a RunSummary instance
func NewRunSummary(
	startAt time.Time,
	terminal cli.Ui,
	singlePackage bool,
	profile string,
	turboVersion string,
	packages []string,
	globalHashSummary *GlobalHashSummary,
	shouldSave bool,
	apiClient *client.APIClient,
	spaceID string,
) Meta {
	executionSummary := newExecutionSummary(startAt, profile)

	return Meta{
		RunSummary: &RunSummary{
			ID:                ksuid.New(),
			Version:           runSummarySchemaVersion,
			ExecutionSummary:  executionSummary,
			TurboVersion:      turboVersion,
			Packages:          packages,
			Tasks:             []*TaskSummary{},
			GlobalHashSummary: globalHashSummary,
		},
		ui:            terminal,
		singlePackage: singlePackage,
		shouldSave:    shouldSave,
		apiClient:     apiClient,
		spaceID:       spaceID,
	}
}

// Close wraps up the RunSummary at the end of a `turbo run`.
func (rsm *Meta) Close(exitCode int, dir turbopath.AbsoluteSystemPath) {
	rsm.RunSummary.ExecutionSummary.exitCode = exitCode
	rsm.RunSummary.ExecutionSummary.endedAt = time.Now()

	summary := rsm.RunSummary
	if err := writeChrometracing(summary.ExecutionSummary.profileFilename, rsm.ui); err != nil {
		rsm.ui.Error(fmt.Sprintf("Error writing tracing data: %v", err))
	}

	// TODO: printing summary to local, writing to disk, and sending to API
	// are all the same thng, we should use a strategy similar to cache save/upload to
	// do this in parallel.

	rsm.printExecutionSummary()

	if rsm.shouldSave {
		if err := rsm.save(dir); err != nil {
			rsm.ui.Warn(fmt.Sprintf("Error writing run summary: %v", err))
		}

		if rsm.spaceID != "" && rsm.apiClient.IsLinked() {
			if err := rsm.record(); err != nil {
				rsm.ui.Warn(fmt.Sprintf("Error recording Run to Vercel: %v", err))
			}
		}
	}
}

// TrackTask makes it possible for the consumer to send information about the execution of a task.
func (summary *RunSummary) TrackTask(taskID string) (func(outcome executionEventName, err error, exitCode *int), *TaskExecutionSummary) {
	return summary.ExecutionSummary.run(taskID)
}

// Save saves the run summary to a file
func (rsm *Meta) save(dir turbopath.AbsoluteSystemPath) error {
	json, err := rsm.FormatJSON()
	if err != nil {
		return err
	}

	// summaryPath will always be relative to the dir passsed in.
	// We don't do a lot of validation, so `../../` paths are allowed
	summaryPath := dir.UntypedJoin(
		filepath.Join(".turbo", "runs"),
		fmt.Sprintf("%s.json", rsm.RunSummary.ID),
	)

	if err := summaryPath.EnsureDir(); err != nil {
		return err
	}

	return summaryPath.WriteFile(json, 0644)
}

// record sends the summary to the API
// TODO: make this work for single package tasks
func (rsm *Meta) record() []error {
	errs := []error{}

	// Right now we'll send the POST to create the Run and the subsequent task payloads
	// when everything after all execution is done, but in the future, this first POST request
	// can happen when the Run actually starts, so we can send updates to Vercel as the tasks progress.
	runsURL := fmt.Sprintf(runsEndpoint, rsm.spaceID)
	var runID string
	payload := newVercelRunCreatePayload(rsm.RunSummary)
	if startPayload, err := json.Marshal(payload); err == nil {
		if resp, err := rsm.apiClient.JSONPost(runsURL, startPayload); err != nil {
			errs = append(errs, err)
		} else {
			vercelRunResponse := &vercelRunResponse{}
			if err := json.Unmarshal(resp, vercelRunResponse); err != nil {
				errs = append(errs, err)
			} else {
				runID = vercelRunResponse.ID
			}
		}
	}

	if runID != "" {
		rsm.postTaskSummaries(runID)

		if donePayload, err := json.Marshal(newVercelDonePayload()); err == nil {
			if _, err := rsm.apiClient.JSONPatch(runsURL, donePayload); err != nil {
				errs = append(errs, err)
			}
		}
	}

	if len(errs) > 0 {
		return errs
	}

	return nil
}

func (rsm *Meta) postTaskSummaries(runID string) {
	// We make at most 8 requests at a time.
	maxParallelRequests := 8
	taskSummaries := rsm.RunSummary.Tasks
	taskCount := len(taskSummaries)
	taskURL := fmt.Sprintf(tasksEndpoint, rsm.spaceID, runID)

	parallelRequestCount := maxParallelRequests
	if taskCount < maxParallelRequests {
		parallelRequestCount = taskCount
	}

	queue := make(chan int, taskCount)

	wg := &sync.WaitGroup{}
	for i := 0; i < parallelRequestCount; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for index := range queue {
				task := taskSummaries[index]
				if taskPayload, err := json.Marshal(task); err == nil {
					if _, err := rsm.apiClient.JSONPost(taskURL, taskPayload); err != nil {
						rsm.ui.Warn(fmt.Sprintf("Eror uploading summary of %s", task.TaskID))
					}
				}
			}
		}()
	}

	for index := range taskSummaries {
		queue <- index
	}
	close(queue)
	wg.Wait()
}

func (summary *RunSummary) normalize() {
	for _, t := range summary.Tasks {
		t.EnvVars.Global = summary.GlobalHashSummary.EnvVars
	}
}
