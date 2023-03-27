// Package run implements `turbo run`
// This file implements the logic for `turbo run --dry`
package run

import (
	gocontext "context"
	"sync"

	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/cache"
	"github.com/vercel/turbo/cli/internal/cmdutil"
	"github.com/vercel/turbo/cli/internal/core"
	"github.com/vercel/turbo/cli/internal/graph"
	"github.com/vercel/turbo/cli/internal/nodes"
	"github.com/vercel/turbo/cli/internal/runsummary"
	"github.com/vercel/turbo/cli/internal/taskhash"
)

// DryRun gets all the info needed from tasks and prints out a summary, but doesn't actually
// execute the task.
func DryRun(
	ctx gocontext.Context,
	g *graph.CompleteGraph,
	rs *runSpec,
	engine *core.Engine,
	taskHashTracker *taskhash.Tracker,
	turboCache cache.Cache,
	base *cmdutil.CmdBase,
	summary runsummary.Meta,
) error {
	defer turboCache.Shutdown()

	dryRunJSON := rs.Opts.runOpts.dryRunJSON

	taskSummaries, err := executeDryRun(
		ctx,
		engine,
		g,
		taskHashTracker,
		rs,
		base,
	)

	if err != nil {
		return err
	}

	// We walk the graph with no concurrency.
	// Populating the cache state is parallelizable.
	// Do this _after_ walking the graph.
	populateCacheState(turboCache, taskSummaries)

	// Assign the Task Summaries to the main summary
	summary.RunSummary.Tasks = taskSummaries

	// Render the dry run as json
	if dryRunJSON {
		rendered, err := summary.FormatJSON()
		if err != nil {
			return err
		}
		base.UI.Output(string(rendered))
		return nil
	}

	return summary.FormatAndPrintText(g.WorkspaceInfos)
}

func executeDryRun(ctx gocontext.Context, engine *core.Engine, g *graph.CompleteGraph, taskHashTracker *taskhash.Tracker, rs *runSpec, base *cmdutil.CmdBase) ([]*runsummary.TaskSummary, error) {
	taskIDs := []*runsummary.TaskSummary{}

	dryRunExecFunc := func(ctx gocontext.Context, packageTask *nodes.PackageTask, taskSummary *runsummary.TaskSummary) error {
		// Assign some fallbacks if they were missing
		if taskSummary.Command == "" {
			taskSummary.Command = runsummary.MissingTaskLabel
		}

		if taskSummary.Framework == "" {
			taskSummary.Framework = runsummary.MissingFrameworkLabel
		}

		taskIDs = append(taskIDs, taskSummary)

		return nil
	}

	// This setup mirrors a real run. We call engine.execute() with
	// a visitor function and some hardcoded execOpts.
	// Note: we do not currently attempt to parallelize the graph walking
	// (as we do in real execution)
	getArgs := func(taskID string) []string {
		return rs.ArgsForTask(taskID)
	}
	visitorFn := g.GetPackageTaskVisitor(ctx, engine.TaskGraph, getArgs, base.Logger, dryRunExecFunc)
	execOpts := core.EngineExecutionOptions{
		Concurrency: 1,
		Parallel:    false,
	}
	errs := engine.Execute(visitorFn, execOpts)

	if len(errs) > 0 {
		for _, err := range errs {
			base.UI.Error(err.Error())
		}
		return nil, errors.New("errors occurred during dry-run graph traversal")
	}

	return taskIDs, nil
}

func populateCacheState(turboCache cache.Cache, taskSummaries []*runsummary.TaskSummary) {
	// We make at most 8 requests at a time for cache state.
	maxParallelRequests := 8
	taskCount := len(taskSummaries)

	parallelRequestCount := maxParallelRequests
	if taskCount < maxParallelRequests {
		parallelRequestCount = taskCount
	}

	queue := make(chan int, taskCount)

	wg := &sync.WaitGroup{}
	for i := 0; i < parallelRequestCount; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for index := range queue {
				task := taskSummaries[index]
				itemStatus := turboCache.Exists(task.Hash)
				task.CacheState = itemStatus
			}
		}()
	}

	for index := range taskSummaries {
		queue <- index
	}
	close(queue)
	wg.Wait()
}
