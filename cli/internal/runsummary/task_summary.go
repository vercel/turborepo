package runsummary

import (
	"github.com/vercel/turbo/cli/internal/cache"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/util"
)

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

// singlePackageTaskSummary is generally identical to TaskSummary, except that it doesn't contain
// references to the workspace names (these show up in TaskID, Dependencies, etc).
// Single Package Repos don't need to identify their "workspace" in a taskID.
type singlePackageTaskSummary struct {
	Task                   string                                `json:"task"`
	Hash                   string                                `json:"hash"`
	CacheState             cache.ItemStatus                      `json:"cacheState"`
	Command                string                                `json:"command"`
	CommandArguments       []string                              `json:"commandArguments"`
	Outputs                []string                              `json:"outputs"`
	ExcludedOutputs        []string                              `json:"excludedOutputs"`
	LogFile                string                                `json:"logFile"`
	Dependencies           []string                              `json:"dependencies"`
	Dependents             []string                              `json:"dependents"`
	ResolvedTaskDefinition *fs.TaskDefinition                    `json:"resolvedTaskDefinition"`
	ExpandedInputs         map[turbopath.AnchoredUnixPath]string `json:"expandedInputs"`
	ExpandedOutputs        []turbopath.AnchoredSystemPath        `json:"expandedOutputs"`
	Framework              string                                `json:"framework"`
	EnvVars                TaskEnvVarSummary                     `json:"environmentVariables"`
	Execution              *TaskExecutionSummary                 `json:"execution,omitempty"`
	ExternalDepsHash       string                                `json:"hashOfExternalDependencies"`
}
