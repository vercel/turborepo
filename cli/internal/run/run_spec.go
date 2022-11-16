// Package run implements `turbo run`
// This file implements some structs for options
package run

import (
	"github.com/vercel/turbo/cli/internal/cache"
	"github.com/vercel/turbo/cli/internal/runcache"
	"github.com/vercel/turbo/cli/internal/scope"
	"github.com/vercel/turbo/cli/internal/util"
)

// runSpec contains the run-specific configuration elements that come from a particular
// invocation of turbo.
type runSpec struct {
	// Target is a list of task that are going to run this time
	// E.g. in `turbo run build lint` Targets will be ["build", "lint"]
	Targets []string

	// FilteredPkgs is the list of packages that are relevant for this run.
	FilteredPkgs util.Set

	// Opts contains various opts, gathered from CLI flags,
	// but bucketed in smaller structs based on what they mean.
	Opts *Opts
}

// ArgsForTask returns the set of args that need to be passed through to the task
func (rs *runSpec) ArgsForTask(task string) []string {
	passThroughArgs := make([]string, 0, len(rs.Opts.runOpts.passThroughArgs))
	for _, target := range rs.Targets {
		if target == task {
			passThroughArgs = append(passThroughArgs, rs.Opts.runOpts.passThroughArgs...)
		}
	}
	return passThroughArgs
}

// Opts holds the current run operations configuration
type Opts struct {
	runOpts      runOpts
	cacheOpts    cache.Opts
	runcacheOpts runcache.Opts
	scopeOpts    scope.Opts
}

// getDefaultOptions returns the default set of Opts for every run
func getDefaultOptions() *Opts {
	return &Opts{
		runOpts: runOpts{
			concurrency: 10,
		},
	}
}

// RunOpts holds the options that control the execution of a turbo run
type runOpts struct {
	// Show a dot graph
	dotGraph string
	// Force execution to be serially one-at-a-time
	concurrency int
	// Whether to execute in parallel (defaults to false)
	parallel bool
	// Whether to emit a perf profile
	profile string
	// If true, continue task executions even if a task fails.
	continueOnError bool
	passThroughArgs []string
	// Restrict execution to only the listed task names. Default false
	only bool
	// Dry run flags
	dryRun     bool
	dryRunJSON bool
	// Graph flags
	graphDot      bool
	graphFile     string
	noDaemon      bool
	singlePackage bool
}
