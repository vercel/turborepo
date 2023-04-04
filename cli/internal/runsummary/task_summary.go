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
	TaskID                 string                                `json:"taskId,omitempty"`
	Task                   string                                `json:"task"`
	Package                string                                `json:"package,omitempty"`
	Hash                   string                                `json:"hash"`
	ExpandedInputs         map[turbopath.AnchoredUnixPath]string `json:"inputs"`
	ExternalDepsHash       string                                `json:"hashOfExternalDependencies"`
	CacheState             cache.ItemStatus                      `json:"cache"`
	Command                string                                `json:"command"`
	CommandArguments       []string                              `json:"cliArguments"`
	Outputs                []string                              `json:"outputs"`
	ExcludedOutputs        []string                              `json:"excludedOutputs"`
	LogFile                string                                `json:"logFile"`
	Dir                    string                                `json:"directory,omitempty"`
	Dependencies           []string                              `json:"dependencies"`
	Dependents             []string                              `json:"dependents"`
	ResolvedTaskDefinition *fs.TaskDefinition                    `json:"resolvedTaskDefinition"`
	ExpandedOutputs        []turbopath.AnchoredSystemPath        `json:"expandedOutputs"`
	Framework              string                                `json:"framework"`
	EnvVars                TaskEnvVarSummary                     `json:"environmentVariables"`
	Execution              *TaskExecutionSummary                 `json:"execution,omitempty"` // omit when it's not set
}

// TaskEnvVarSummary contains the environment variables that impacted a task's hash
type TaskEnvVarSummary struct {
	Configured []string `json:"configured"`
	Inferred   []string `json:"inferred"`
	Global     []string `json:"global"`
}

// cleanForSinglePackage converts a TaskSummary to remove references to workspaces
func (ts *TaskSummary) cleanForSinglePackage() {
	dependencies := make([]string, len(ts.Dependencies))
	for i, dependency := range ts.Dependencies {
		dependencies[i] = util.StripPackageName(dependency)
	}
	dependents := make([]string, len(ts.Dependents))
	for i, dependent := range ts.Dependents {
		dependents[i] = util.StripPackageName(dependent)
	}

	ts.Task = util.RootTaskTaskName(ts.TaskID)
	ts.Dependencies = dependencies
	ts.Dependents = dependents
	ts.TaskID = ""
	ts.Dir = ""
	ts.Package = ""
}
