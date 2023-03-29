package run

import (
	gocontext "context"
	"fmt"
	"log"
	"os"
	"os/exec"
	"strings"
	"sync"
	"time"

	"github.com/fatih/color"
	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
	"github.com/pkg/errors"
	"github.com/pyr-sh/dag"
	"github.com/vercel/turbo/cli/internal/cache"
	"github.com/vercel/turbo/cli/internal/cmdutil"
	"github.com/vercel/turbo/cli/internal/colorcache"
	"github.com/vercel/turbo/cli/internal/core"
	"github.com/vercel/turbo/cli/internal/graph"
	"github.com/vercel/turbo/cli/internal/logstreamer"
	"github.com/vercel/turbo/cli/internal/nodes"
	"github.com/vercel/turbo/cli/internal/packagemanager"
	"github.com/vercel/turbo/cli/internal/process"
	"github.com/vercel/turbo/cli/internal/runcache"
	"github.com/vercel/turbo/cli/internal/runsummary"
	"github.com/vercel/turbo/cli/internal/spinner"
	"github.com/vercel/turbo/cli/internal/taskhash"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/ui"
)

// RealRun executes a set of tasks
func RealRun(
	ctx gocontext.Context,
	g *graph.CompleteGraph,
	rs *runSpec,
	engine *core.Engine,
	taskHashTracker *taskhash.Tracker,
	turboCache cache.Cache,
	packagesInScope []string,
	base *cmdutil.CmdBase,
	runSummary runsummary.Meta,
	packageManager *packagemanager.PackageManager,
	processes *process.Manager,
) error {
	singlePackage := rs.Opts.runOpts.singlePackage

	if singlePackage {
		base.UI.Output(fmt.Sprintf("%s %s", ui.Dim("• Running"), ui.Dim(ui.Bold(strings.Join(rs.Targets, ", ")))))
	} else {
		base.UI.Output(fmt.Sprintf(ui.Dim("• Packages in scope: %v"), strings.Join(packagesInScope, ", ")))
		base.UI.Output(fmt.Sprintf("%s %s %s", ui.Dim("• Running"), ui.Dim(ui.Bold(strings.Join(rs.Targets, ", "))), ui.Dim(fmt.Sprintf("in %v packages", rs.FilteredPkgs.Len()))))
	}

	// Log whether remote cache is enabled
	useHTTPCache := !rs.Opts.cacheOpts.SkipRemote
	if useHTTPCache {
		base.UI.Info(ui.Dim("• Remote caching enabled"))
	} else {
		base.UI.Info(ui.Dim("• Remote caching disabled"))
	}

	defer func() {
		_ = spinner.WaitFor(ctx, turboCache.Shutdown, base.UI, "...writing to cache...", 1500*time.Millisecond)
	}()
	colorCache := colorcache.New()

	runCache := runcache.New(turboCache, base.RepoRoot, rs.Opts.runcacheOpts, colorCache)

	ec := &execContext{
		colorCache:      colorCache,
		runSummary:      runSummary,
		rs:              rs,
		ui:              &cli.ConcurrentUi{Ui: base.UI},
		runCache:        runCache,
		logger:          base.Logger,
		packageManager:  packageManager,
		processes:       processes,
		taskHashTracker: taskHashTracker,
		repoRoot:        base.RepoRoot,
		isSinglePackage: singlePackage,
	}

	// run the thing
	execOpts := core.EngineExecutionOptions{
		Parallel:    rs.Opts.runOpts.parallel,
		Concurrency: rs.Opts.runOpts.concurrency,
	}

	mu := sync.Mutex{}
	taskSummaries := []*runsummary.TaskSummary{}
	execFunc := func(ctx gocontext.Context, packageTask *nodes.PackageTask, taskSummary *runsummary.TaskSummary) error {
		deps := engine.TaskGraph.DownEdges(packageTask.TaskID)
		// deps here are passed in to calculate the task hash
		taskExecutionSummary, err := ec.exec(ctx, packageTask, deps)
		if err != nil {
			return err
		}

		// taskExecutionSummary will be nil if the task never executed
		// (i.e. if the workspace didn't implement the script corresponding to the task)
		// We don't need to collect any of the outputs or execution if the task didn't execute.
		if taskExecutionSummary != nil {
			taskSummary.ExpandedOutputs = taskHashTracker.GetExpandedOutputs(taskSummary.TaskID)
			taskSummary.Execution = taskExecutionSummary
			taskSummary.CacheState = taskHashTracker.GetCacheStatus(taskSummary.TaskID)

			// lock since multiple things to be appending to this array at the same time
			mu.Lock()
			taskSummaries = append(taskSummaries, taskSummary)
			// not using defer, just release the lock
			mu.Unlock()
		}

		return nil
	}

	getArgs := func(taskID string) []string {
		return rs.ArgsForTask(taskID)
	}

	visitorFn := g.GetPackageTaskVisitor(ctx, engine.TaskGraph, getArgs, base.Logger, execFunc)
	errs := engine.Execute(visitorFn, execOpts)

	// Track if we saw any child with a non-zero exit code
	exitCode := 0
	exitCodeErr := &process.ChildExit{}

	// Assign tasks after execution
	runSummary.RunSummary.Tasks = taskSummaries

	for _, err := range errs {
		if errors.As(err, &exitCodeErr) {
			// If a process gets killed via a signal, Go reports it's exit code as -1.
			// We take the absolute value of the exit code so we don't select '0' as
			// the greatest exit code.
			childExit := exitCodeErr.ExitCode
			if childExit < 0 {
				childExit = -childExit
			}
			if childExit > exitCode {
				exitCode = childExit
			}
		} else if exitCode == 0 {
			// We hit some error, it shouldn't be exit code 0
			exitCode = 1
		}
		base.UI.Error(err.Error())
	}

	// When continue on error is enabled don't register failed tasks as errors
	// and instead must inspect the task summaries.
	if ec.rs.Opts.runOpts.continueOnError {
		for _, summary := range runSummary.RunSummary.Tasks {
			if childExit := summary.Execution.ExitCode(); childExit != nil {
				childExit := *childExit
				if childExit < 0 {
					childExit = -childExit
				}
				if childExit > exitCode {
					exitCode = childExit
				}
			}
		}
	}

	runSummary.Close(exitCode, base.RepoRoot)

	if exitCode != 0 {
		return &process.ChildExit{
			ExitCode: exitCode,
		}
	}
	return nil
}

type execContext struct {
	colorCache      *colorcache.ColorCache
	runSummary      runsummary.Meta
	rs              *runSpec
	ui              cli.Ui
	runCache        *runcache.RunCache
	logger          hclog.Logger
	packageManager  *packagemanager.PackageManager
	processes       *process.Manager
	taskHashTracker *taskhash.Tracker
	repoRoot        turbopath.AbsoluteSystemPath
	isSinglePackage bool
}

func (ec *execContext) logError(log hclog.Logger, prefix string, err error) {
	ec.logger.Error(prefix, "error", err)

	if prefix != "" {
		prefix += ": "
	}

	ec.ui.Error(fmt.Sprintf("%s%s%s", ui.ERROR_PREFIX, prefix, color.RedString(" %v", err)))
}

func (ec *execContext) exec(ctx gocontext.Context, packageTask *nodes.PackageTask, deps dag.Set) (*runsummary.TaskExecutionSummary, error) {
	// Setup tracer. Every time tracer() is called the taskExecutionSummary's duration is updated
	// So make sure to call it before returning.
	tracer, taskExecutionSummary := ec.runSummary.RunSummary.TrackTask(packageTask.TaskID)

	progressLogger := ec.logger.Named("")
	progressLogger.Debug("start")

	passThroughArgs := ec.rs.ArgsForTask(packageTask.Task)
	hash := packageTask.Hash
	ec.logger.Debug("task hash", "value", hash)
	// TODO(gsoltis): if/when we fix https://github.com/vercel/turbo/issues/937
	// the following block should never get hit. In the meantime, keep it after hashing
	// so that downstream tasks can count on the hash existing
	//
	// bail if the script doesn't exist
	if packageTask.Command == "" {
		progressLogger.Debug("no task in package, skipping")
		progressLogger.Debug("done", "status", "skipped", "duration", taskExecutionSummary.Duration)
		// Return nil here because there was no execution, so there is no task execution summary
		return nil, nil
	}

	var prefix string
	var prettyPrefix string
	if ec.rs.Opts.runOpts.logPrefix == "none" {
		prefix = ""
	} else {
		prefix = packageTask.OutputPrefix(ec.isSinglePackage)
	}

	prettyPrefix = ec.colorCache.PrefixWithColor(packageTask.PackageName, prefix)

	// Cache ---------------------------------------------
	taskCache := ec.runCache.TaskCache(packageTask, hash)
	// Create a logger for replaying
	prefixedUI := &cli.PrefixedUi{
		Ui:           ec.ui,
		OutputPrefix: prettyPrefix,
		InfoPrefix:   prettyPrefix,
		ErrorPrefix:  prettyPrefix,
		WarnPrefix:   prettyPrefix,
	}
	cacheStatus, err := taskCache.RestoreOutputs(ctx, prefixedUI, progressLogger)
	ec.taskHashTracker.SetCacheStatus(packageTask.TaskID, cacheStatus)

	hit := cacheStatus.Local || cacheStatus.Remote
	if err != nil {
		prefixedUI.Error(fmt.Sprintf("error fetching from cache: %s", err))
	} else if hit {
		ec.taskHashTracker.SetExpandedOutputs(packageTask.TaskID, taskCache.ExpandedOutputs)
		// We only cache successful executions, so we can assume this is a successCode exit.
		tracer(runsummary.TargetCached, nil, &successCode)
		return taskExecutionSummary, nil
	}

	// Setup command execution
	argsactual := append([]string{"run"}, packageTask.Task)
	if len(passThroughArgs) > 0 {
		// This will be either '--' or a typed nil
		argsactual = append(argsactual, ec.packageManager.ArgSeparator...)
		argsactual = append(argsactual, passThroughArgs...)
	}

	cmd := exec.Command(ec.packageManager.Command, argsactual...)
	cmd.Dir = packageTask.Pkg.Dir.ToSystemPath().RestoreAnchor(ec.repoRoot).ToString()
	envs := fmt.Sprintf("TURBO_HASH=%v", hash)
	cmd.Env = append(os.Environ(), envs)

	// Setup stdout/stderr
	// If we are not caching anything, then we don't need to write logs to disk
	// be careful about this conditional given the default of cache = true
	writer, err := taskCache.OutputWriter(prettyPrefix)
	if err != nil {
		tracer(runsummary.TargetBuildFailed, err, nil)

		ec.logError(progressLogger, prettyPrefix, err)
		if !ec.rs.Opts.runOpts.continueOnError {
			return nil, errors.Wrapf(err, "failed to capture outputs for \"%v\"", packageTask.TaskID)
		}
	}

	// Create a logger
	logger := log.New(writer, "", 0)
	// Setup a streamer that we'll pipe cmd.Stdout to
	logStreamerOut := logstreamer.NewLogstreamer(logger, prettyPrefix, false)
	// Setup a streamer that we'll pipe cmd.Stderr to.
	logStreamerErr := logstreamer.NewLogstreamer(logger, prettyPrefix, false)
	cmd.Stderr = logStreamerErr
	cmd.Stdout = logStreamerOut
	// Flush/Reset any error we recorded
	logStreamerErr.FlushRecord()
	logStreamerOut.FlushRecord()

	closeOutputs := func() error {
		var closeErrors []error

		if err := logStreamerOut.Close(); err != nil {
			closeErrors = append(closeErrors, errors.Wrap(err, "log stdout"))
		}
		if err := logStreamerErr.Close(); err != nil {
			closeErrors = append(closeErrors, errors.Wrap(err, "log stderr"))
		}

		if err := writer.Close(); err != nil {
			closeErrors = append(closeErrors, errors.Wrap(err, "log file"))
		}
		if len(closeErrors) > 0 {
			msgs := make([]string, len(closeErrors))
			for i, err := range closeErrors {
				msgs[i] = err.Error()
			}
			return fmt.Errorf("could not flush log output: %v", strings.Join(msgs, ", "))
		}
		return nil
	}

	// Run the command
	if err := ec.processes.Exec(cmd); err != nil {
		// close off our outputs. We errored, so we mostly don't care if we fail to close
		_ = closeOutputs()
		// if we already know we're in the process of exiting,
		// we don't need to record an error to that effect.
		if errors.Is(err, process.ErrClosing) {
			return taskExecutionSummary, nil
		}

		// If the error we got is a ChildExit, it will have an ExitCode field
		// Pass that along into the tracer.
		var e *process.ChildExit
		if errors.As(err, &e) {
			tracer(runsummary.TargetBuildFailed, err, &e.ExitCode)
		} else {
			// If it wasn't a ChildExit, and something else went wrong, we don't have an exitCode
			tracer(runsummary.TargetBuildFailed, err, nil)
		}

		progressLogger.Error(fmt.Sprintf("Error: command finished with error: %v", err))
		if !ec.rs.Opts.runOpts.continueOnError {
			prefixedUI.Error(fmt.Sprintf("ERROR: command finished with error: %s", err))
			ec.processes.Close()
		} else {
			prefixedUI.Warn("command finished with error, but continuing...")
			// Set to nil so we don't short-circuit any other execution
			err = nil
		}

		// If there was an error, flush the buffered output
		taskCache.OnError(prefixedUI, progressLogger)

		return taskExecutionSummary, err
	}

	// Close off our outputs and cache them
	if err := closeOutputs(); err != nil {
		ec.logError(progressLogger, "", err)
	} else {
		if err = taskCache.SaveOutputs(ctx, progressLogger, prefixedUI, int(taskExecutionSummary.Duration.Milliseconds())); err != nil {
			ec.logError(progressLogger, "", fmt.Errorf("error caching output: %w", err))
		} else {
			ec.taskHashTracker.SetExpandedOutputs(packageTask.TaskID, taskCache.ExpandedOutputs)
		}
	}

	// Clean up tracing
	tracer(runsummary.TargetBuilt, nil, &successCode)
	progressLogger.Debug("done", "status", "complete", "duration", taskExecutionSummary.Duration)
	return taskExecutionSummary, nil
}

var successCode = 0
