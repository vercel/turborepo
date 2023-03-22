// Package runsummary implements structs that report on a `turbo run` and `turbo run --dry`
package runsummary

import (
	"fmt"
	"path/filepath"
	"time"

	"github.com/mitchellh/cli"
	"github.com/segmentio/ksuid"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// MissingTaskLabel is printed when a package is missing a definition for a task that is supposed to run
// E.g. if `turbo run build --dry` is run, and package-a doesn't define a `build` script in package.json,
// the RunSummary will print this, instead of the script (e.g. `next build`).
const MissingTaskLabel = "<NONEXISTENT>"

// MissingFrameworkLabel is a string to identify when a workspace doesn't detect a framework
const MissingFrameworkLabel = "<NO FRAMEWORK DETECTED>"

const runSummarySchemaVersion = "0"

// Meta is a wrapper around the serializable RunSummary, with some extra information
// about the Run and references to other things that we need.
type Meta struct {
	RunSummary    *RunSummary
	ui            cli.Ui
	singlePackage bool
	shouldSave    bool
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
	}
}

// Close wraps up the RunSummary at the end of a `turbo run`.
func (rsm *Meta) Close(dir turbopath.AbsoluteSystemPath) {
	summary := rsm.RunSummary
	if err := writeChrometracing(summary.ExecutionSummary.profileFilename, rsm.ui); err != nil {
		rsm.ui.Error(fmt.Sprintf("Error writing tracing data: %v", err))
	}

	rsm.printExecutionSummary()

	if rsm.shouldSave {
		if err := rsm.save(dir); err != nil {
			rsm.ui.Warn(fmt.Sprintf("Error writing run summary: %v", err))
		}
	}
}

// TrackTask makes it possible for the consumer to send information about the execution of a task.
func (summary *RunSummary) TrackTask(taskID string) (func(outcome executionEventName, err error), *TaskExecutionSummary) {
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

func (summary *RunSummary) normalize() {
	for _, t := range summary.Tasks {
		t.EnvVars.Global = summary.GlobalHashSummary.EnvVars
	}
}
