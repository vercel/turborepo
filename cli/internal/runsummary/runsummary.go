// Package runsummary implements structs that report on a `turbo run` and `turbo run --dry`
package runsummary

import (
	"time"

	"github.com/vercel/turbo/cli/internal/cache"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/packagemanager"
	"github.com/vercel/turbo/cli/internal/runcache"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/util"
)

// MissingFrameworkLabel is a string to identify when a workspace doesn't detect a framework
const MissingFrameworkLabel = "<NO FRAMEWORK DETECTED>"

// TaskExecutionSummary contains data about the actual execution of a task
type TaskExecutionSummary struct {
	Start    time.Time     `json:"start"`
	Duration time.Duration `json:"duration"`
	Label    string        `json:"-"`      // Target which has just changed. Omit from JSOn
	Status   string        `json:"status"` // Its current status
	Err      error         `json:"error"`  // Error, only populated for failure statuses
}

// DryRunSummary contains a summary of the packages and tasks that would run
// if the --dry flag had not been passed
type DryRunSummary struct {
	TurboVersion      string                         `json:"turboVersion"`
	GlobalHashSummary *GlobalHashSummary             `json:"globalHashSummary"`
	PackageManager    *packagemanager.PackageManager `json:"packageManager"`
	Packages          []string                       `json:"packages"`
	ExitCode          int                            `json:"exitCode"`
	Tasks             []TaskSummary                  `json:"tasks"`
}

// GlobalHashSummary contains the pieces of data that impacted the global hash (then then impacted the task hash)
type GlobalHashSummary struct {
	GlobalFileHashMap    map[turbopath.AnchoredUnixPath]string `json:"globalFileHashMap"`
	RootExternalDepsHash string                                `json:"rootExternalDepsHash"`
	GlobalCacheKey       string                                `json:"globalCacheKey"`
	Pipeline             fs.PristinePipeline                   `json:"pipeline"`
}

// NewGlobalHashSummary tasks an anonymous struct and prepares it for reporting
// TODO(mehulkar): the upstream struct that is passed in cannot become a named struct
// because it will change the global hash, invalidating all caches. We should do this
// only when we are ok with invalidating the global hash.
func NewGlobalHashSummary(globalFileHashMap map[turbopath.AnchoredUnixPath]string, rootExternalDepsHash string, hashedSortedEnvPairs []string, globalCacheKey string, pipeline fs.PristinePipeline) *GlobalHashSummary {
	// TODO(mehulkar): Add hashedSortedEnvPairs in here, but redact the values
	return &GlobalHashSummary{
		GlobalFileHashMap:    globalFileHashMap,
		RootExternalDepsHash: rootExternalDepsHash,
		GlobalCacheKey:       globalCacheKey,
		Pipeline:             pipeline,
	}
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
	RunSummary             *TaskExecutionSummary                 `json:"taskSummary"`
	ExpandedInputs         map[turbopath.AnchoredUnixPath]string `json:"expandedInputs"`
	ExpandedOutputs        *runcache.ExpandedOutputs             `json:"expandedOutputs"`
	Framework              string                                `json:"framework"`
	EnvVars                TaskEnvVarSummary                     `json:"environmentVariables"`
}

// SinglePackageTaskSummary is generally identally to TaskSummary except that it doesn't contain
// references to the package names (e.g. TaskID, Dependencies, etc)
// Single Packages don't need to identify their "workspace" in a taskID
type SinglePackageTaskSummary struct {
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
	RunSummary             *TaskExecutionSummary                 `json:"taskSummary"`
	ExpandedInputs         map[turbopath.AnchoredUnixPath]string `json:"expandedInputs"`
	ExpandedOutputs        *runcache.ExpandedOutputs             `json:"expandedOutputs"`
	Framework              string                                `json:"framework"`
	EnvVars                TaskEnvVarSummary                     `json:"environmentVariables"`
}

// ToSinglePackageTask converts a TaskSummary into a SinglePackageTaskSummary
func (ht *TaskSummary) ToSinglePackageTask() SinglePackageTaskSummary {
	dependencies := make([]string, len(ht.Dependencies))
	for i, depencency := range ht.Dependencies {
		dependencies[i] = util.StripPackageName(depencency)
	}
	dependents := make([]string, len(ht.Dependents))
	for i, dependent := range ht.Dependents {
		dependents[i] = util.StripPackageName(dependent)
	}

	return SinglePackageTaskSummary{
		Task:                   util.RootTaskTaskName(ht.TaskID),
		Hash:                   ht.Hash,
		CacheState:             ht.CacheState,
		Command:                ht.Command,
		Outputs:                ht.Outputs,
		LogFile:                ht.LogFile,
		Dependencies:           dependencies,
		Dependents:             dependents,
		ResolvedTaskDefinition: ht.ResolvedTaskDefinition,
		RunSummary:             ht.RunSummary,
		Framework:              ht.Framework,
		ExpandedInputs:         ht.ExpandedInputs,
		EnvVars:                ht.EnvVars,
	}
}

// TaskEnvVarSummary contains the environment variables that impacted a task's hash
type TaskEnvVarSummary struct {
	Configured []string `json:"configured"`
	Inferred   []string `json:"inferred"`
}

// SinglePackageDryRunSummary is the same as DryRunSummary with some adjustments
// to the internal struct for a single package. It's likely that we can use the
// same struct for Single Package repos in the future.
type SinglePackageDryRunSummary struct {
	Tasks []SinglePackageTaskSummary `json:"tasks"`
}
