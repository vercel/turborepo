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
	"github.com/vercel/turbo/cli/internal/util"
	"github.com/vercel/turbo/cli/internal/workspace"
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

type runType int

const (
	runTypeReal runType = iota
	runTypeDryText
	runTypeDryJSON
)

// Meta is a wrapper around the serializable RunSummary, with some extra information
// about the Run and references to other things that we need.
type Meta struct {
	RunSummary    *RunSummary
	ui            cli.Ui
	repoRoot      turbopath.AbsoluteSystemPath // used to write run summary
	singlePackage bool
	shouldSave    bool
	apiClient     *client.APIClient
	spaceID       string
	runType       runType
}

// RunSummary contains a summary of what happens in the `turbo run` command and why.
type RunSummary struct {
	ID                ksuid.KSUID        `json:"id"`
	Version           string             `json:"version"`
	TurboVersion      string             `json:"turboVersion"`
	GlobalHashSummary *GlobalHashSummary `json:"globalHashSummary"`
	Packages          []string           `json:"packages,omitempty"`
	ExecutionSummary  *executionSummary  `json:"executionSummary,omitempty"`
	Tasks             []*TaskSummary     `json:"tasks"`
}

// NewRunSummary returns a RunSummary instance
func NewRunSummary(
	startAt time.Time,
	ui cli.Ui,
	repoRoot turbopath.AbsoluteSystemPath,
	turboVersion string,
	apiClient *client.APIClient,
	runOpts util.RunOpts,
	packages []string,
	globalHashSummary *GlobalHashSummary,
) Meta {
	singlePackage := runOpts.SinglePackage
	profile := runOpts.Profile
	shouldSave := runOpts.Summarize
	spaceID := runOpts.ExperimentalSpaceID

	runType := runTypeReal
	if runOpts.DryRun {
		runType = runTypeDryText
		if runOpts.DryRunJSON {
			runType = runTypeDryJSON
		}
	}

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
		ui:            ui,
		runType:       runType,
		repoRoot:      repoRoot,
		singlePackage: singlePackage,
		shouldSave:    shouldSave,
		apiClient:     apiClient,
		spaceID:       spaceID,
	}
}

// getPath returns a path to where the runSummary is written.
// The returned path will always be relative to the dir passsed in.
// We don't do a lot of validation, so `../../` paths are allowed.
func (rsm *Meta) getPath() turbopath.AbsoluteSystemPath {
	filename := fmt.Sprintf("%s.json", rsm.RunSummary.ID)
	return rsm.repoRoot.UntypedJoin(filepath.Join(".turbo", "runs"), filename)
}

// Close wraps up the RunSummary at the end of a `turbo run`.
func (rsm *Meta) Close(exitCode int, workspaceInfos workspace.Catalog) error {
	if rsm.runType == runTypeDryJSON || rsm.runType == runTypeDryText {
		return rsm.closeDryRun(workspaceInfos)
	}

	rsm.RunSummary.ExecutionSummary.exitCode = exitCode
	rsm.RunSummary.ExecutionSummary.endedAt = time.Now()

	summary := rsm.RunSummary
	if err := writeChrometracing(summary.ExecutionSummary.profileFilename, rsm.ui); err != nil {
		rsm.ui.Error(fmt.Sprintf("Error writing tracing data: %v", err))
	}

	// TODO: printing summary to local, writing to disk, and sending to API
	// are all the same thng, we should use a strategy similar to cache save/upload to
	// do this in parallel.

	// Otherwise, attempt to save the summary
	// Warn on the error, but we don't need to throw an error
	if rsm.shouldSave {
		if err := rsm.save(); err != nil {
			rsm.ui.Warn(fmt.Sprintf("Error writing run summary: %v", err))
		}
	}

	rsm.printExecutionSummary()

	if rsm.shouldSave {
		if rsm.spaceID != "" && rsm.apiClient.IsLinked() {
			if err := rsm.record(); err != nil {
				rsm.ui.Warn(fmt.Sprintf("Error recording Run to Vercel: %v", err))
			}
		}
	}

	return nil
}

// closeDryRun wraps up the Run Summary at the end of `turbo run --dry`.
// Ideally this should be inlined into Close(), but RunSummary doesn't currently
// have context about whether a run was real or dry.
func (rsm *Meta) closeDryRun(workspaceInfos workspace.Catalog) error {
	// Render the dry run as json
	if rsm.runType == runTypeDryJSON {
		rendered, err := rsm.FormatJSON()
		if err != nil {
			return err
		}

		rsm.ui.Output(string(rendered))
		return nil
	}

	return rsm.FormatAndPrintText(workspaceInfos)
}

// TrackTask makes it possible for the consumer to send information about the execution of a task.
func (summary *RunSummary) TrackTask(taskID string) (func(outcome executionEventName, err error, exitCode *int), *TaskExecutionSummary) {
	return summary.ExecutionSummary.run(taskID)
}

// Save saves the run summary to a file
func (rsm *Meta) save() error {
	json, err := rsm.FormatJSON()
	if err != nil {
		return err
	}

	// summaryPath will always be relative to the dir passsed in.
	// We don't do a lot of validation, so `../../` paths are allowed
	summaryPath := rsm.getPath()

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

		if donePayload, err := json.Marshal(newVercelDonePayload(rsm.RunSummary)); err == nil {
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
