// Package runsummary implements structs that report on a `turbo run` and `turbo run --dry`
package runsummary

import (
	"context"
	"fmt"
	"path/filepath"
	"time"

	"github.com/mitchellh/cli"
	"github.com/segmentio/ksuid"
	"github.com/vercel/turbo/cli/internal/ci"
	"github.com/vercel/turbo/cli/internal/client"
	"github.com/vercel/turbo/cli/internal/env"
	"github.com/vercel/turbo/cli/internal/spinner"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/util"
	"github.com/vercel/turbo/cli/internal/workspace"
)

// MissingTaskLabel is printed when a package is missing a definition for a task that is supposed to run
// E.g. if `turbo run build --dry` is run, and package-a doesn't define a `build` script in package.json,
// the RunSummary will print this, instead of the script (e.g. `next build`).
const MissingTaskLabel = "<NONEXISTENT>"

// NoFrameworkDetected is a string to identify when a workspace doesn't detect a framework
const NoFrameworkDetected = "<NO FRAMEWORK DETECTED>"

// FrameworkDetectionSkipped is a string to identify when framework detection was skipped
const FrameworkDetectionSkipped = "<FRAMEWORK DETECTION SKIPPED>"

const runSummarySchemaVersion = "1"

type runType int

const (
	runTypeReal runType = iota
	runTypeDryText
	runTypeDryJSON
)

// Meta is a wrapper around the serializable RunSummary, with some extra information
// about the Run and references to other things that we need.
type Meta struct {
	RunSummary         *RunSummary
	ui                 cli.Ui
	repoRoot           turbopath.AbsoluteSystemPath // used to write run summary
	repoPath           turbopath.RelativeSystemPath
	singlePackage      bool
	shouldSave         bool
	spacesClient       *spacesClient
	runType            runType
	synthesizedCommand string
}

// RunSummary contains a summary of what happens in the `turbo run` command and why.
type RunSummary struct {
	ID                 ksuid.KSUID        `json:"id"`
	Version            string             `json:"version"`
	TurboVersion       string             `json:"turboVersion"`
	GlobalHashSummary  *GlobalHashSummary `json:"globalCacheInputs"`
	Packages           []string           `json:"packages"`
	EnvMode            util.EnvMode       `json:"envMode"`
	FrameworkInference bool               `json:"frameworkInference"`
	ExecutionSummary   *executionSummary  `json:"execution,omitempty"`
	Tasks              []*TaskSummary     `json:"tasks"`
	User               string             `json:"user"`
	SCM                *scmState          `json:"scm"`
}

// NewRunSummary returns a RunSummary instance
func NewRunSummary(
	startAt time.Time,
	ui cli.Ui,
	repoRoot turbopath.AbsoluteSystemPath,
	repoPath turbopath.RelativeSystemPath,
	turboVersion string,
	apiClient *client.APIClient,
	runOpts util.RunOpts,
	packages []string,
	globalEnvMode util.EnvMode,
	envAtExecutionStart env.EnvironmentVariableMap,
	globalHashSummary *GlobalHashSummary,
	synthesizedCommand string,
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

	executionSummary := newExecutionSummary(synthesizedCommand, repoPath, startAt, profile)

	rsm := Meta{
		RunSummary: &RunSummary{
			ID:                 ksuid.New(),
			Version:            runSummarySchemaVersion,
			ExecutionSummary:   executionSummary,
			TurboVersion:       turboVersion,
			Packages:           packages,
			EnvMode:            globalEnvMode,
			FrameworkInference: runOpts.FrameworkInference,
			Tasks:              []*TaskSummary{},
			GlobalHashSummary:  globalHashSummary,
			SCM:                getSCMState(envAtExecutionStart, repoRoot),
			User:               getUser(envAtExecutionStart, repoRoot),
		},
		ui:                 ui,
		runType:            runType,
		repoRoot:           repoRoot,
		singlePackage:      singlePackage,
		shouldSave:         shouldSave,
		synthesizedCommand: synthesizedCommand,
	}

	rsm.spacesClient = newSpacesClient(spaceID, apiClient, ui)
	if rsm.spacesClient.enabled {
		go rsm.spacesClient.start()
		rsm.spacesClient.createRun(&rsm)
	}

	return rsm
}

// getPath returns a path to where the runSummary is written.
// The returned path will always be relative to the dir passsed in.
// We don't do a lot of validation, so `../../` paths are allowed.
func (rsm *Meta) getPath() turbopath.AbsoluteSystemPath {
	filename := fmt.Sprintf("%s.json", rsm.RunSummary.ID)
	return rsm.repoRoot.UntypedJoin(filepath.Join(".turbo", "runs"), filename)
}

// Close wraps up the RunSummary at the end of a `turbo run`.
func (rsm *Meta) Close(ctx context.Context, exitCode int, workspaceInfos workspace.Catalog) error {
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
	if rsm.spacesClient.enabled {
		rsm.sendToSpace(ctx)
	} else {
		// Print any errors if the client is not enabled, since it could have
		// been disabled at runtime due to an issue.
		rsm.spacesClient.printErrors()
	}

	return nil
}

func (rsm *Meta) sendToSpace(ctx context.Context) {
	rsm.spacesClient.finishRun(rsm)
	func() {
		_ = spinner.WaitFor(ctx, rsm.spacesClient.Close, rsm.ui, "...sending run summary...", 1000*time.Millisecond)
	}()

	rsm.spacesClient.printErrors()

	url := rsm.spacesClient.run.URL
	if url != "" {
		rsm.ui.Output(fmt.Sprintf("Run: %s", url))
		rsm.ui.Output("")
	}
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

func (summary *RunSummary) getFailedTasks() []*TaskSummary {
	failed := []*TaskSummary{}

	for _, t := range summary.Tasks {
		if *t.Execution.exitCode != 0 {
			failed = append(failed, t)
		}
	}
	return failed
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

// CloseTask posts the result of the Task to Spaces
func (rsm *Meta) CloseTask(task *TaskSummary) {
	if rsm.spacesClient.enabled {
		rsm.spacesClient.postTask(task)
	}
}

func getUser(envVars env.EnvironmentVariableMap, dir turbopath.AbsoluteSystemPath) string {
	var username string

	if ci.IsCi() {
		vendor := ci.Info()
		username = envVars[vendor.UsernameEnvVar]
	}

	return username
}
