package runsummary

import (
	"github.com/vercel/turbo/cli/internal/cache"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// singlePackageRunSummary is the same as RunSummary with some adjustments
// to the internal struct for a single package. It's likely that we can use the
// same struct for Single Package repos in the future.
type singlePackageRunSummary struct {
	Tasks []singlePackageTaskSummary `json:"tasks"`
}

// singlePackageTaskSummary is generally identical to TaskSummary, except that it doesn't contain
// references to the workspace names (these show up in TaskID, Dependencies, etc).
// Single Package Repos don't need to identify their "workspace" in a taskID.
type singlePackageTaskSummary struct {
	Task                   string                                `json:"task"`
	Hash                   string                                `json:"hash"`
	CacheState             cache.ItemStatus                      `json:"cacheState"`
	Command                string                                `json:"command"`
	Outputs                []string                              `json:"outputs"`
	ExcludedOutputs        []string                              `json:"excludedOutputs"`
	LogFile                string                                `json:"logFile"`
	Dependencies           []string                              `json:"dependencies"`
	Dependents             []string                              `json:"dependents"`
	ResolvedTaskDefinition *fs.TaskDefinition                    `json:"resolvedTaskDefinition"`
	ExpandedInputs         map[turbopath.AnchoredUnixPath]string `json:"expandedInputs"`
	Framework              string                                `json:"framework"`
	EnvVars                TaskEnvVarSummary                     `json:"environmentVariables"`
	Execution              *TaskExecutionSummary                 `json:"execution"`
}
