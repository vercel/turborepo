// Package run implements `turbo run`
// This file implements some structs for options
package run

import (
	"strings"

	"github.com/vercel/turbo/cli/internal/cache"
	"github.com/vercel/turbo/cli/internal/client"
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
	passThroughArgs := make([]string, 0, len(rs.Opts.runOpts.PassThroughArgs))
	for _, target := range rs.Targets {
		if target == task {
			passThroughArgs = append(passThroughArgs, rs.Opts.runOpts.PassThroughArgs...)
		}
	}
	return passThroughArgs
}

// Opts holds the current run operations configuration
type Opts struct {
	runOpts      util.RunOpts
	cacheOpts    cache.Opts
	clientOpts   client.Opts
	runcacheOpts runcache.Opts
	scopeOpts    scope.Opts
}

// SynthesizeCommand produces a command that produces an equivalent set of packages, tasks,
// and task arguments to what the current set of opts selects.
func (o *Opts) SynthesizeCommand(tasks []string) string {
	cmd := "turbo run"
	cmd += " " + strings.Join(tasks, " ")
	for _, filterPattern := range o.scopeOpts.FilterPatterns {
		cmd += " --filter=" + filterPattern
	}
	for _, filterPattern := range o.scopeOpts.LegacyFilter.AsFilterPatterns() {
		cmd += " --filter=" + filterPattern
	}
	if o.runOpts.Parallel {
		cmd += " --parallel"
	}
	if o.runOpts.ContinueOnError {
		cmd += " --continue"
	}
	if o.runOpts.DryRun {
		if o.runOpts.DryRunJSON {
			cmd += " --dry=json"
		} else {
			cmd += " --dry"
		}
	}
	if len(o.runOpts.PassThroughArgs) > 0 {
		cmd += " -- " + strings.Join(o.runOpts.PassThroughArgs, " ")
	}
	return cmd
}

// getDefaultOptions returns the default set of Opts for every run
func getDefaultOptions() *Opts {
	return &Opts{
		runOpts: util.RunOpts{
			Concurrency: 10,
		},
		clientOpts: client.Opts{
			Timeout: client.ClientTimeout,
		},
	}
}
