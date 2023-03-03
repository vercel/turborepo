// Package runsummary implements structs that report on a `turbo run` and `turbo run --dry`
package runsummary

import (
	"time"

	"github.com/vercel/turbo/cli/internal/cache"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/runcache"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/util"
)

// MissingFrameworkLabel is a string to identify when a workspace doesn't detect a framework
const MissingFrameworkLabel = "<NO FRAMEWORK DETECTED>"

// RunSummary contains a summary of what happens in the `turbo run` command and why.
type RunSummary struct {
	TurboVersion      string             `json:"turboVersion"`
	GlobalHashSummary *GlobalHashSummary `json:"globalHashSummary"`
	Packages          []string           `json:"packages"`
	Tasks             []TaskSummary      `json:"tasks"`
	ExitCode          int                `json:"exitCode"`
}

// TaskExecutionSummary contains data about the actual execution of a task
type TaskExecutionSummary struct {
	Start    time.Time     `json:"start"`
	Duration time.Duration `json:"duration"`
	Label    string        `json:"-"`      // Target which has just changed. Omit from JSOn
	Status   string        `json:"status"` // Its current status
	Err      error         `json:"error"`  // Error, only populated for failure statuses
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
	Outputs                []string                              `json:"outputs"`
	ExcludedOutputs        []string                              `json:"excludedOutputs"`
	LogFile                string                                `json:"logFile"`
	Dir                    string                                `json:"directory"`
	Dependencies           []string                              `json:"dependencies"`
	Dependents             []string                              `json:"dependents"`
	ResolvedTaskDefinition *fs.TaskDefinition                    `json:"resolvedTaskDefinition"`
	ExpandedInputs         map[turbopath.AnchoredUnixPath]string `json:"expandedInputs"`
	Framework              string                                `json:"framework"`
	EnvVars                TaskEnvVarSummary                     `json:"environmentVariables"`
	RunSummary             *TaskExecutionSummary                 `json:"taskSummary"`
	ExpandedOutputs        *runcache.ExpandedOutputs             `json:"expandedOuputs"`
}

// TaskEnvVarSummary contains the environment variables that impacted a task's hash
type TaskEnvVarSummary struct {
	Configured []string `json:"configured"`
	Inferred   []string `json:"inferred"`
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
		Outputs:                ht.Outputs,
		LogFile:                ht.LogFile,
		Dependencies:           dependencies,
		Dependents:             dependents,
		ResolvedTaskDefinition: ht.ResolvedTaskDefinition,
		Framework:              ht.Framework,
		ExpandedInputs:         ht.ExpandedInputs,
		EnvVars:                ht.EnvVars,
	}
}
