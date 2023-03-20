// Package runsummary implements structs that report on a `turbo run` and `turbo run --dry`
package runsummary

import (
	"fmt"
	"path/filepath"
	"time"

	"github.com/mitchellh/cli"
	"github.com/segmentio/ksuid"
	"github.com/vercel/turbo/cli/internal/cache"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/util"
)

// MissingTaskLabel is printed when a package is missing a definition for a task that is supposed to run
// E.g. if `turbo run build --dry` is run, and package-a doesn't define a `build` script in package.json,
// the RunSummary will print this, instead of the script (e.g. `next build`).
const MissingTaskLabel = "<NONEXISTENT>"

// MissingFrameworkLabel is a string to identify when a workspace doesn't detect a framework
const MissingFrameworkLabel = "<NO FRAMEWORK DETECTED>"

const runSummarySchemaVersion = "0"

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

// NewRunSummary returns a RunSummary instance
func NewRunSummary(startAt time.Time, profile string, turboVersion string, packages []string, globalHashSummary *GlobalHashSummary) *RunSummary {
	executionSummary := newExecutionSummary(startAt, profile)

	return &RunSummary{
		ID:                ksuid.New(),
		Version:           runSummarySchemaVersion,
		ExecutionSummary:  executionSummary,
		TurboVersion:      turboVersion,
		Packages:          packages,
		Tasks:             []*TaskSummary{},
		GlobalHashSummary: globalHashSummary,
	}
}

// Close wraps up the RunSummary at the end of a `turbo run`.
func (summary *RunSummary) Close(terminal cli.Ui) {
	if err := writeChrometracing(summary.ExecutionSummary.profileFilename, terminal); err != nil {
		terminal.Error(fmt.Sprintf("Error writing tracing data: %v", err))
	}

	summary.printExecutionSummary(terminal)
}

// TrackTask makes it possible for the consumer to send information about the execution of a task.
func (summary *RunSummary) TrackTask(taskID string) (func(outcome executionEventName, err error), *TaskExecutionSummary) {
	return summary.ExecutionSummary.run(taskID)
}

func (summary *RunSummary) normalize() {
	for _, t := range summary.Tasks {
		t.EnvVars.Global = summary.GlobalHashSummary.EnvVars
	}
}

// Save saves the run summary to a file
func (summary *RunSummary) Save(dir turbopath.AbsoluteSystemPath, singlePackage bool) error {
	json, err := summary.FormatJSON(singlePackage)
	if err != nil {
		return err
	}

	// summaryPath will always be relative to the dir passsed in.
	// We don't do a lot of validation, so `../../` paths are allowed
	summaryPath := dir.UntypedJoin(
		filepath.Join(".turbo", "runs"),
		fmt.Sprintf("%s.json", summary.ID),
	)

	if err := summaryPath.EnsureDir(); err != nil {
		return err
	}

	return summaryPath.WriteFile(json, 0644)
}

// TaskSummary contains information about the task that was about to run
// TODO(mehulkar): `Outputs` and `ExcludedOutputs` are slightly redundant
// as the information is also available in ResolvedTaskDefinition. We could remove them
// and favor a version of Outputs that is the fully expanded list of files.
type TaskSummary struct {
	TaskID                 string                                `json:"taskId"`
	Task                   string                                `json:"task"`
	Package                string                                `json:"package"`
	Hash                   string                                `json:"hash"`
	CacheState             cache.ItemStatus                      `json:"cacheState"`
	Command                string                                `json:"command"`
	CommandArguments       []string                              `json:"commandArguments"`
	Outputs                []string                              `json:"outputs"`
	ExcludedOutputs        []string                              `json:"excludedOutputs"`
	LogFile                string                                `json:"logFile"`
	Dir                    string                                `json:"directory"`
	Dependencies           []string                              `json:"dependencies"`
	Dependents             []string                              `json:"dependents"`
	ResolvedTaskDefinition *fs.TaskDefinition                    `json:"resolvedTaskDefinition"`
	ExpandedInputs         map[turbopath.AnchoredUnixPath]string `json:"expandedInputs"`
	ExpandedOutputs        []turbopath.AnchoredSystemPath        `json:"expandedOutputs"`
	Framework              string                                `json:"framework"`
	EnvVars                TaskEnvVarSummary                     `json:"environmentVariables"`
	Execution              *TaskExecutionSummary                 `json:"execution,omitempty"` // omit when it's not set
	ExternalDepsHash       string                                `json:"hashOfExternalDependencies"`
}

// TaskEnvVarSummary contains the environment variables that impacted a task's hash
type TaskEnvVarSummary struct {
	Configured []string `json:"configured"`
	Inferred   []string `json:"inferred"`
	Global     []string `json:"global"`
}

// toSinglePackageTask converts a TaskSummary into a singlePackageTaskSummary
func (ht *TaskSummary) toSinglePackageTask() singlePackageTaskSummary {
	dependencies := make([]string, len(ht.Dependencies))
	for i, depencency := range ht.Dependencies {
		dependencies[i] = util.StripPackageName(depencency)
	}
	dependents := make([]string, len(ht.Dependents))
	for i, dependent := range ht.Dependents {
		dependents[i] = util.StripPackageName(dependent)
	}

	return singlePackageTaskSummary{
		Task:                   util.RootTaskTaskName(ht.TaskID),
		Hash:                   ht.Hash,
		CacheState:             ht.CacheState,
		Command:                ht.Command,
		CommandArguments:       ht.CommandArguments,
		Outputs:                ht.Outputs,
		LogFile:                ht.LogFile,
		Dependencies:           dependencies,
		Dependents:             dependents,
		ResolvedTaskDefinition: ht.ResolvedTaskDefinition,
		Framework:              ht.Framework,
		ExpandedInputs:         ht.ExpandedInputs,
		ExpandedOutputs:        ht.ExpandedOutputs,
		EnvVars:                ht.EnvVars,
		Execution:              ht.Execution,
		ExternalDepsHash:       ht.ExternalDepsHash,
	}
}
