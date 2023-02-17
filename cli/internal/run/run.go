package run

import (
	gocontext "context"
	"fmt"
	"os"
	"sort"
	"sync"
	"time"

	"github.com/vercel/turbo/cli/internal/analytics"
	"github.com/vercel/turbo/cli/internal/cache"
	"github.com/vercel/turbo/cli/internal/cmdutil"
	"github.com/vercel/turbo/cli/internal/context"
	"github.com/vercel/turbo/cli/internal/core"
	"github.com/vercel/turbo/cli/internal/daemon"
	"github.com/vercel/turbo/cli/internal/daemonclient"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/graph"
	"github.com/vercel/turbo/cli/internal/process"
	"github.com/vercel/turbo/cli/internal/scm"
	"github.com/vercel/turbo/cli/internal/scope"
	"github.com/vercel/turbo/cli/internal/signals"
	"github.com/vercel/turbo/cli/internal/taskhash"
	"github.com/vercel/turbo/cli/internal/turbostate"
	"github.com/vercel/turbo/cli/internal/ui"
	"github.com/vercel/turbo/cli/internal/util"

	"github.com/pkg/errors"
)

var _cmdLong = `
Run tasks across projects in your monorepo.

By default, turbo executes tasks in topological order (i.e.
dependencies first) and then caches the results. Re-running commands for
tasks already in the cache will skip re-execution and immediately move
artifacts from the cache into the correct output folders (as if the task
occurred again).

Arguments passed after '--' will be passed through to the named tasks.
`

// ExecuteRun executes the run command
func ExecuteRun(ctx gocontext.Context, helper *cmdutil.Helper, signalWatcher *signals.Watcher, args *turbostate.ParsedArgsFromRust) error {
	base, err := helper.GetCmdBase(args)
	if err != nil {
		return err
	}
	tasks := args.Command.Run.Tasks
	passThroughArgs := args.Command.Run.PassThroughArgs
	if len(tasks) == 0 {
		return errors.New("at least one task must be specified")
	}
	opts, err := optsFromArgs(args)
	if err != nil {
		return err
	}

	opts.runOpts.passThroughArgs = passThroughArgs
	run := configureRun(base, opts, signalWatcher)
	if err := run.run(ctx, tasks); err != nil {
		base.LogError("run failed: %v", err)
		return err
	}
	return nil
}

func optsFromArgs(args *turbostate.ParsedArgsFromRust) (*Opts, error) {
	runPayload := args.Command.Run

	opts := getDefaultOptions()
	// aliases := make(map[string]string)
	scope.OptsFromArgs(&opts.scopeOpts, args)

	// Cache flags
	opts.cacheOpts.SkipFilesystem = runPayload.RemoteOnly
	opts.cacheOpts.OverrideDir = runPayload.CacheDir
	opts.cacheOpts.Workers = runPayload.CacheWorkers
	opts.runOpts.logPrefix = runPayload.LogPrefix

	// Runcache flags
	opts.runcacheOpts.SkipReads = runPayload.Force
	opts.runcacheOpts.SkipWrites = runPayload.NoCache

	if runPayload.OutputLogs != "" {
		err := opts.runcacheOpts.SetTaskOutputMode(runPayload.OutputLogs)
		if err != nil {
			return nil, err
		}
	}

	// Run flags
	if runPayload.Concurrency != "" {
		concurrency, err := util.ParseConcurrency(runPayload.Concurrency)
		if err != nil {
			return nil, err
		}
		opts.runOpts.concurrency = concurrency
	}
	opts.runOpts.parallel = runPayload.Parallel
	opts.runOpts.profile = runPayload.Profile
	opts.runOpts.continueOnError = runPayload.ContinueExecution
	opts.runOpts.only = runPayload.Only
	opts.runOpts.noDaemon = runPayload.NoDaemon
	opts.runOpts.singlePackage = args.Command.Run.SinglePackage

	// See comment on Graph in turbostate.go for an explanation on Graph's representation.
	// If flag is passed...
	if runPayload.Graph != nil {
		// If no value is attached, we print to stdout
		if *runPayload.Graph == "" {
			opts.runOpts.graphDot = true
		} else {
			// Otherwise, we emit to the file name attached as value
			opts.runOpts.graphDot = false
			opts.runOpts.graphFile = *runPayload.Graph
		}
	}

	if runPayload.DryRun != "" {
		opts.runOpts.dryRunJSON = runPayload.DryRun == _dryRunJSONValue

		if runPayload.DryRun == _dryRunTextValue || runPayload.DryRun == _dryRunJSONValue {
			opts.runOpts.dryRun = true
		} else {
			return nil, fmt.Errorf("invalid dry-run mode: %v", runPayload.DryRun)
		}
	}

	return opts, nil
}

func configureRun(base *cmdutil.CmdBase, opts *Opts, signalWatcher *signals.Watcher) *run {
	if os.Getenv("TURBO_FORCE") == "true" {
		opts.runcacheOpts.SkipReads = true
	}

	if os.Getenv("TURBO_REMOTE_ONLY") == "true" {
		opts.cacheOpts.SkipFilesystem = true
	}

	processes := process.NewManager(base.Logger.Named("processes"))
	signalWatcher.AddOnClose(processes.Close)
	return &run{
		base:      base,
		opts:      opts,
		processes: processes,
	}
}

type run struct {
	base      *cmdutil.CmdBase
	opts      *Opts
	processes *process.Manager
}

func (r *run) run(ctx gocontext.Context, targets []string) error {
	startAt := time.Now()
	packageJSONPath := r.base.RepoRoot.UntypedJoin("package.json")
	rootPackageJSON, err := fs.ReadPackageJSON(packageJSONPath)
	if err != nil {
		return fmt.Errorf("failed to read package.json: %w", err)
	}

	var pkgDepGraph *context.Context
	if r.opts.runOpts.singlePackage {
		pkgDepGraph, err = context.SinglePackageGraph(r.base.RepoRoot, rootPackageJSON)
	} else {
		pkgDepGraph, err = context.BuildPackageGraph(r.base.RepoRoot, rootPackageJSON)
	}
	if err != nil {
		var warnings *context.Warnings
		if errors.As(err, &warnings) {
			r.base.LogWarning("Issues occurred when constructing package graph. Turbo will function, but some features may not be available", err)
		} else {
			return err
		}
	}

	if ui.IsCI && !r.opts.runOpts.noDaemon {
		r.base.Logger.Info("skipping turbod since we appear to be in a non-interactive context")
	} else if !r.opts.runOpts.noDaemon {
		turbodClient, err := daemon.GetClient(ctx, r.base.RepoRoot, r.base.Logger, r.base.TurboVersion, daemon.ClientOpts{})
		if err != nil {
			r.base.LogWarning("", errors.Wrap(err, "failed to contact turbod. Continuing in standalone mode"))
		} else {
			defer func() { _ = turbodClient.Close() }()
			r.base.Logger.Debug("running in daemon mode")
			daemonClient := daemonclient.New(turbodClient)
			r.opts.runcacheOpts.OutputWatcher = daemonClient
		}
	}

	if err := util.ValidateGraph(&pkgDepGraph.WorkspaceGraph); err != nil {
		return errors.Wrap(err, "Invalid package dependency graph")
	}

	// TODO: consolidate some of these arguments
	// Note: not all properties are set here. GlobalHash and Pipeline keys are set later
	g := &graph.CompleteGraph{
		WorkspaceGraph:  pkgDepGraph.WorkspaceGraph,
		WorkspaceInfos:  pkgDepGraph.WorkspaceInfos,
		RootNode:        pkgDepGraph.RootNode,
		TaskDefinitions: map[string]*fs.TaskDefinition{},
		RepoRoot:        r.base.RepoRoot,
	}

	turboJSON, err := g.GetTurboConfigFromWorkspace(util.RootPkgName, r.opts.runOpts.singlePackage)
	if err != nil {
		return err
	}

	// TODO: these values come from a config file, hopefully viper can help us merge these
	r.opts.cacheOpts.RemoteCacheOpts = turboJSON.RemoteCacheOptions

	pipeline := turboJSON.Pipeline
	g.Pipeline = pipeline
	scmInstance, err := scm.FromInRepo(r.base.RepoRoot)
	if err != nil {
		if errors.Is(err, scm.ErrFallback) {
			r.base.LogWarning("", err)
		} else {
			return errors.Wrap(err, "failed to create SCM")
		}
	}
	filteredPkgs, isAllPackages, err := scope.ResolvePackages(&r.opts.scopeOpts, r.base.RepoRoot, scmInstance, pkgDepGraph, r.base.UI, r.base.Logger)
	if err != nil {
		return errors.Wrap(err, "failed to resolve packages to run")
	}
	if isAllPackages {
		// if there is a root task for any of our targets, we need to add it
		for _, target := range targets {
			key := util.RootTaskID(target)
			if _, ok := pipeline[key]; ok {
				filteredPkgs.Add(util.RootPkgName)
				// we only need to know we're running a root task once to add it for consideration
				break
			}
		}
	}

	globalHash, err := calculateGlobalHash(
		r.base.RepoRoot,
		rootPackageJSON,
		pipeline,
		turboJSON.GlobalEnv,
		turboJSON.GlobalDeps,
		pkgDepGraph.PackageManager,
		pkgDepGraph.Lockfile,
		r.base.Logger,
		os.Environ(),
	)

	g.GlobalHash = globalHash

	if err != nil {
		return fmt.Errorf("failed to calculate global hash: %v", err)
	}
	r.base.Logger.Debug("global hash", "value", globalHash)
	r.base.Logger.Debug("local cache folder", "path", r.opts.cacheOpts.OverrideDir)

	rs := &runSpec{
		Targets:      targets,
		FilteredPkgs: filteredPkgs,
		Opts:         r.opts,
	}
	packageManager := pkgDepGraph.PackageManager

	engine, err := buildTaskGraphEngine(
		g,
		rs,
		r.opts.runOpts.singlePackage,
	)

	if err != nil {
		return errors.Wrap(err, "error preparing engine")
	}

	tracker := taskhash.NewTracker(
		g.RootNode,
		g.GlobalHash,
		// TODO(mehulkar): remove g,Pipeline, because we need to get task definitions from CompleteGaph instead
		g.Pipeline,
		g.WorkspaceInfos,
	)

	err = tracker.CalculateFileHashes(
		engine.TaskGraph.Vertices(),
		rs.Opts.runOpts.concurrency,
		r.base.RepoRoot,
		g,
	)

	if err != nil {
		return errors.Wrap(err, "error hashing package files")
	}

	// If we are running in parallel, then we remove all the edges in the graph
	// except for the root. Rebuild the task graph for backwards compatibility.
	// We still use dependencies specified by the pipeline configuration.
	if rs.Opts.runOpts.parallel {
		for _, edge := range g.WorkspaceGraph.Edges() {
			if edge.Target() != g.RootNode {
				g.WorkspaceGraph.RemoveEdge(edge)
			}
		}
		engine, err = buildTaskGraphEngine(
			g,
			rs,
			r.opts.runOpts.singlePackage,
		)
		if err != nil {
			return errors.Wrap(err, "error preparing engine")
		}
	}

	// Graph Run
	if rs.Opts.runOpts.graphFile != "" || rs.Opts.runOpts.graphDot {
		return GraphRun(ctx, rs, engine, r.base)
	}

	packagesInScope := rs.FilteredPkgs.UnsafeListOfStrings()
	sort.Strings(packagesInScope)
	// Initiate analytics and cache
	analyticsClient := r.initAnalyticsClient(ctx)
	defer analyticsClient.CloseWithTimeout(50 * time.Millisecond)
	turboCache, err := r.initCache(ctx, rs, analyticsClient)

	if err != nil {
		if errors.Is(err, cache.ErrNoCachesEnabled) {
			r.base.UI.Warn("No caches are enabled. You can try \"turbo login\", \"turbo link\", or ensuring you are not passing --remote-only to enable caching")
		} else {
			return errors.Wrap(err, "failed to set up caching")
		}
	}

	// Dry Run
	if rs.Opts.runOpts.dryRun {
		// dryRunSummary contains information that is statically analyzable about
		// the tasks that we expect to run based on the user command.
		// Currently, we only emit this on dry runs, but it may be useful for real runs later also.
		summary := &dryRunSummary{
			Packages: packagesInScope,
			Tasks:    []taskSummary{},
		}

		return DryRun(
			ctx,
			g,
			rs,
			engine,
			tracker,
			turboCache,
			r.base,
			summary,
		)
	}

	// RunState captures the runtime results for this run (e.g. timings of each task and profile)
	runState := NewRunState(startAt, r.opts.runOpts.profile)
	// Regular run
	return RealRun(
		ctx,
		g,
		rs,
		engine,
		tracker,
		turboCache,
		packagesInScope,
		r.base,
		// Extra arg only for regular runs, dry-run doesn't get this
		packageManager,
		r.processes,
		runState,
	)
}

func (r *run) initAnalyticsClient(ctx gocontext.Context) analytics.Client {
	apiClient := r.base.APIClient
	var analyticsSink analytics.Sink
	if apiClient.IsLinked() {
		analyticsSink = apiClient
	} else {
		r.opts.cacheOpts.SkipRemote = true
		analyticsSink = analytics.NullSink
	}
	analyticsClient := analytics.NewClient(ctx, analyticsSink, r.base.Logger.Named("analytics"))
	return analyticsClient
}

func (r *run) initCache(ctx gocontext.Context, rs *runSpec, analyticsClient analytics.Client) (cache.Cache, error) {
	apiClient := r.base.APIClient
	// Theoretically this is overkill, but bias towards not spamming the console
	once := &sync.Once{}

	return cache.New(rs.Opts.cacheOpts, r.base.RepoRoot, apiClient, analyticsClient, func(_cache cache.Cache, err error) {
		// Currently the HTTP Cache is the only one that can be disabled.
		// With a cache system refactor, we might consider giving names to the caches so
		// we can accurately report them here.
		once.Do(func() {
			r.base.LogWarning("Remote Caching is unavailable", err)
		})
	})
}

func buildTaskGraphEngine(
	g *graph.CompleteGraph,
	rs *runSpec,
	isSinglePackage bool,
) (*core.Engine, error) {
	engine := core.NewEngine(g, isSinglePackage)

	// Note: g.Pipeline is a map, but this for loop only cares about the keys
	for taskName := range g.Pipeline {
		engine.AddTask(taskName)
	}

	if err := engine.Prepare(&core.EngineBuildingOptions{
		Packages:  rs.FilteredPkgs.UnsafeListOfStrings(),
		TaskNames: rs.Targets,
		TasksOnly: rs.Opts.runOpts.only,
	}); err != nil {
		return nil, err
	}

	// Check for cycles in the DAG.
	if err := util.ValidateGraph(engine.TaskGraph); err != nil {
		return nil, fmt.Errorf("Invalid task dependency graph:\n%v", err)
	}

	// Check that no tasks would be blocked by a persistent task
	if err := engine.ValidatePersistentDependencies(g); err != nil {
		return nil, fmt.Errorf("Invalid persistent task dependency:\n%v", err)
	}

	return engine, nil
}

// dry run custom flag
// NOTE: These *must* be kept in sync with the corresponding Rust
// enum definitions in shim/src/commands/mod.rs
const (
	_dryRunJSONValue = "Json"
	_dryRunTextValue = "Text"
)
