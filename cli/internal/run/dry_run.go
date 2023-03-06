// Package run implements `turbo run`
// This file implements the logic for `turbo run --dry`
package run

import (
	gocontext "context"
	"fmt"
	"path/filepath"
	"regexp"

	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/cache"
	"github.com/vercel/turbo/cli/internal/cmdutil"
	"github.com/vercel/turbo/cli/internal/core"
	"github.com/vercel/turbo/cli/internal/graph"
	"github.com/vercel/turbo/cli/internal/nodes"
	"github.com/vercel/turbo/cli/internal/runsummary"
	"github.com/vercel/turbo/cli/internal/taskhash"
	"github.com/vercel/turbo/cli/internal/util"
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
	summary *runsummary.RunSummary,
) error {
	defer turboCache.Shutdown()

	dryRunJSON := rs.Opts.runOpts.dryRunJSON
	singlePackage := rs.Opts.runOpts.singlePackage

	taskSummaries, err := executeDryRun(
		ctx,
		engine,
		g,
		taskHashTracker,
		rs,
		base,
		turboCache,
	)

	if err != nil {
		return err
	}

	// Assign the Task Summaries to the main summary
	summary.Tasks = taskSummaries

	// Render the dry run as json
	if dryRunJSON {
		rendered, err := summary.FormatJSON(singlePackage)
		if err != nil {
			return err
		}
		base.UI.Output(string(rendered))
		return nil
	}

	return summary.FormatAndPrintText(base.UI, g.WorkspaceInfos, singlePackage)
}

func executeDryRun(ctx gocontext.Context, engine *core.Engine, g *graph.CompleteGraph, taskHashTracker *taskhash.Tracker, rs *runSpec, base *cmdutil.CmdBase, turboCache cache.Cache) ([]*runsummary.TaskSummary, error) {
	taskIDs := []*runsummary.TaskSummary{}

	dryRunExecFunc := func(ctx gocontext.Context, packageTask *nodes.PackageTask, taskSummary *runsummary.TaskSummary) error {
		isRootTask := packageTask.PackageName == util.RootPkgName
		if isRootTask && commandLooksLikeTurbo(taskSummary.Command) {
			return fmt.Errorf("root task %v (%v) looks like it invokes turbo and might cause a loop", packageTask.Task, taskSummary.Command)
		}

		ancestors, err := engine.GetTaskGraphAncestors(packageTask.TaskID)
		if err != nil {
			return err
		}

		descendents, err := engine.GetTaskGraphDescendants(packageTask.TaskID)
		if err != nil {
			return err
		}

		itemStatus, err := turboCache.Exists(packageTask.Hash)
		if err != nil {
			return err
		}

		// Assign some fallbacks if they were missing
		if taskSummary.Command == "" {
			taskSummary.Command = runsummary.MissingTaskLabel
		}

		if taskSummary.Framework == "" {
			taskSummary.Framework = runsummary.MissingFrameworkLabel
		}

		taskSummary.CacheState = itemStatus  // TODO(mehulkar): Move this to PackageTask
		taskSummary.Dependencies = ancestors // TODO(mehulkar): Move this to PackageTask
		taskSummary.Dependents = descendents // TODO(mehulkar): Move this to PackageTask

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

var _isTurbo = regexp.MustCompile(fmt.Sprintf("(?:^|%v|\\s)turbo(?:$|\\s)", regexp.QuoteMeta(string(filepath.Separator))))

func commandLooksLikeTurbo(command string) bool {
	return _isTurbo.MatchString(command)
}
