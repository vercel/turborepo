package run

import (
	gocontext "context"
	"fmt"
	"os"
	"sort"
	"sync"
	"time"

	"github.com/spf13/cobra"
	"github.com/spf13/pflag"
	"github.com/vercel/turbo/cli/internal/analytics"
	"github.com/vercel/turbo/cli/internal/cache"
	"github.com/vercel/turbo/cli/internal/cmdutil"
	"github.com/vercel/turbo/cli/internal/config"
	"github.com/vercel/turbo/cli/internal/context"
	"github.com/vercel/turbo/cli/internal/core"
	"github.com/vercel/turbo/cli/internal/daemon"
	"github.com/vercel/turbo/cli/internal/daemonclient"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/graph"
	"github.com/vercel/turbo/cli/internal/packagemanager"
	"github.com/vercel/turbo/cli/internal/process"
	"github.com/vercel/turbo/cli/internal/runcache"
	"github.com/vercel/turbo/cli/internal/scm"
	"github.com/vercel/turbo/cli/internal/scope"
	"github.com/vercel/turbo/cli/internal/signals"
	"github.com/vercel/turbo/cli/internal/taskhash"
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

// GetCmd returns the run command
func GetCmd(helper *cmdutil.Helper, signalWatcher *signals.Watcher) *cobra.Command {
	var opts *Opts
	var flags *pflag.FlagSet

	cmd := &cobra.Command{
		Use:                   "run <task> [...<task>] [<flags>] -- <args passed to tasks>",
		Short:                 "Run tasks across projects in your monorepo",
		Long:                  _cmdLong,
		SilenceUsage:          true,
		SilenceErrors:         true,
		DisableFlagsInUseLine: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			flagSet := config.FlagSet{FlagSet: cmd.Flags()}
			base, err := helper.GetCmdBase(flagSet)
			if err != nil {
				return err
			}
			tasks, passThroughArgs := parseTasksAndPassthroughArgs(args, flags)
			if len(tasks) == 0 {
				return errors.New("at least one task must be specified")
			}

			_, packageMode := packagemanager.InferRoot(base.RepoRoot)

			opts.runOpts.singlePackage = packageMode == packagemanager.Single
			opts.runOpts.passThroughArgs = passThroughArgs

			run := configureRun(base, opts, signalWatcher)
			ctx := cmd.Context()
			if err := run.run(ctx, tasks); err != nil {
				base.LogError("run failed: %v", err)
				return err
			}
			return nil
		},
	}

	flags = cmd.Flags()
	opts = optsFromFlags(flags)
	return cmd
}

func parseTasksAndPassthroughArgs(remainingArgs []string, flags *pflag.FlagSet) ([]string, []string) {
	if argSplit := flags.ArgsLenAtDash(); argSplit != -1 {
		return remainingArgs[:argSplit], remainingArgs[argSplit:]
	}
	return remainingArgs, nil
}

func optsFromFlags(flags *pflag.FlagSet) *Opts {
	opts := getDefaultOptions()
	aliases := make(map[string]string)
	scope.AddFlags(&opts.scopeOpts, flags)
	addRunOpts(&opts.runOpts, flags, aliases)
	cache.AddFlags(&opts.cacheOpts, flags)
	runcache.AddFlags(&opts.runcacheOpts, flags)
	flags.SetNormalizeFunc(func(f *pflag.FlagSet, name string) pflag.NormalizedName {
		if alias, ok := aliases[name]; ok {
			return pflag.NormalizedName(alias)
		}
		return pflag.NormalizedName(name)
	})
	return opts
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
	turboJSON, err := fs.LoadTurboConfig(r.base.RepoRoot, rootPackageJSON, r.opts.runOpts.singlePackage)
	if err != nil {
		return err
	}

	// TODO: these values come from a config file, hopefully viper can help us merge these
	r.opts.cacheOpts.RemoteCacheOpts = turboJSON.RemoteCacheOptions

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

	if err := util.ValidateGraph(&pkgDepGraph.TopologicalGraph); err != nil {
		return errors.Wrap(err, "Invalid package dependency graph")
	}

	pipeline := turboJSON.Pipeline
	if err := validateTasks(pipeline, targets); err != nil {
		return err
	}

	scmInstance, err := scm.FromInRepo(r.base.RepoRoot)
	if err != nil {
		if errors.Is(err, scm.ErrFallback) {
			r.base.LogWarning("", err)
		} else {
			return errors.Wrap(err, "failed to create SCM")
		}
	}
	filteredPkgs, isAllPackages, err := scope.ResolvePackages(&r.opts.scopeOpts, r.base.RepoRoot.ToStringDuringMigration(), scmInstance, pkgDepGraph, r.base.UI, r.base.Logger)
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
	if err != nil {
		return fmt.Errorf("failed to calculate global hash: %v", err)
	}
	r.base.Logger.Debug("global hash", "value", globalHash)
	r.base.Logger.Debug("local cache folder", "path", r.opts.cacheOpts.OverrideDir)

	// TODO: consolidate some of these arguments
	g := &graph.CompleteGraph{
		TopologicalGraph: pkgDepGraph.TopologicalGraph,
		Pipeline:         pipeline,
		PackageInfos:     pkgDepGraph.PackageInfos,
		GlobalHash:       globalHash,
		RootNode:         pkgDepGraph.RootNode,
	}
	rs := &runSpec{
		Targets:      targets,
		FilteredPkgs: filteredPkgs,
		Opts:         r.opts,
	}
	packageManager := pkgDepGraph.PackageManager

	vertexSet := make(util.Set)
	for _, v := range g.TopologicalGraph.Vertices() {
		vertexSet.Add(v)
	}

	engine, err := buildTaskGraphEngine(g, rs)

	if err != nil {
		return errors.Wrap(err, "error preparing engine")
	}
	tracker := taskhash.NewTracker(g.RootNode, g.GlobalHash, g.Pipeline, g.PackageInfos)
	err = tracker.CalculateFileHashes(engine.TaskGraph.Vertices(), rs.Opts.runOpts.concurrency, r.base.RepoRoot)
	if err != nil {
		return errors.Wrap(err, "error hashing package files")
	}

	// If we are running in parallel, then we remove all the edges in the graph
	// except for the root. Rebuild the task graph for backwards compatibility.
	// We still use dependencies specified by the pipeline configuration.
	if rs.Opts.runOpts.parallel {
		for _, edge := range g.TopologicalGraph.Edges() {
			if edge.Target() != g.RootNode {
				g.TopologicalGraph.RemoveEdge(edge)
			}
		}
		engine, err = buildTaskGraphEngine(g, rs)
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
		return DryRun(
			ctx,
			g,
			rs,
			engine,
			tracker,
			turboCache,
			packagesInScope,
			r.base,
		)
	}

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
		startAt,
		r.processes,
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

func buildTaskGraphEngine(g *graph.CompleteGraph, rs *runSpec) (*core.Engine, error) {
	engine := core.NewEngine(&g.TopologicalGraph)

	for taskName, taskDefinition := range g.Pipeline {
		deps := make(util.Set)

		isPackageTask := util.IsPackageTask(taskName)

		for _, dependency := range taskDefinition.TaskDependencies {
			// If the current task is a workspace-specific task (including root Task)
			// and its dependency is _also_ a workspace-specific task, we need to add
			// a reference to this dependency directly into the engine.
			// TODO @mehulkar: Why do we need this?
			if isPackageTask && util.IsPackageTask(dependency) {
				err := engine.AddDep(dependency, taskName)
				if err != nil {
					return nil, err
				}
			} else {
				// For non-workspace-specific dependencies, we attach a reference to
				// the task that is added into the engine.
				deps.Add(dependency)
			}
		}

		topoDeps := util.SetFromStrings(taskDefinition.TopologicalDependencies)
		engine.AddTask(&core.Task{
			Name:       taskName,
			TopoDeps:   topoDeps,
			Deps:       deps,
			Persistent: taskDefinition.Persistent,
		})
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

var (
	_profileHelp = `File to write turbo's performance profile output into.
You can load the file up in chrome://tracing to see
which parts of your build were slow.`
	_continueHelp = `Continue execution even if a task exits with an error
or non-zero exit code. The default behavior is to bail`
	_dryRunHelp = `List the packages in scope and the tasks that would be run,
but don't actually run them. Passing --dry=json or
--dry-run=json will render the output in JSON format.`
	_graphHelp = `Generate a graph of the task execution and output to a file when a filename is specified (.svg, .png, .jpg, .pdf, .json, .html).
Outputs dot graph to stdout when if no filename is provided`
	_concurrencyHelp = `Limit the concurrency of task execution. Use 1 for serial (i.e. one-at-a-time) execution.`
	_parallelHelp    = `Execute all tasks in parallel.`
	_onlyHelp        = `Run only the specified tasks, not their dependencies.`
)

func addRunOpts(opts *runOpts, flags *pflag.FlagSet, aliases map[string]string) {
	flags.AddFlag(&pflag.Flag{
		Name:     "concurrency",
		Usage:    _concurrencyHelp,
		DefValue: "10",
		Value: &util.ConcurrencyValue{
			Value: &opts.concurrency,
		},
	})
	flags.BoolVar(&opts.parallel, "parallel", false, _parallelHelp)
	flags.StringVar(&opts.profile, "profile", "", _profileHelp)
	flags.BoolVar(&opts.continueOnError, "continue", false, _continueHelp)
	flags.BoolVar(&opts.only, "only", false, _onlyHelp)
	flags.BoolVar(&opts.noDaemon, "no-daemon", false, "Run without using turbo's daemon process")
	flags.BoolVar(&opts.singlePackage, "single-package", false, "Run turbo in single-package mode")
	// This is a no-op flag, we don't need it anymore
	flags.Bool("experimental-use-daemon", false, "Use the experimental turbo daemon")
	if err := flags.MarkHidden("experimental-use-daemon"); err != nil {
		panic(err)
	}
	if err := flags.MarkHidden("only"); err != nil {
		// fail fast if we've messed up our flag configuration
		panic(err)
	}
	if err := flags.MarkHidden("single-package"); err != nil {
		panic(err)
	}
	aliases["dry"] = "dry-run"
	flags.AddFlag(&pflag.Flag{
		Name:        "dry-run",
		Usage:       _dryRunHelp,
		DefValue:    "",
		NoOptDefVal: _dryRunNoValue,
		Value:       &dryRunValue{opts: opts},
	})
	flags.AddFlag(&pflag.Flag{
		Name:        "graph",
		Usage:       _graphHelp,
		DefValue:    "",
		NoOptDefVal: _graphNoValue,
		Value:       &graphValue{opts: opts},
	})
}

const (
	_graphText      = "graph"
	_graphNoValue   = "<output filename>"
	_graphTextValue = "true"
)

// graphValue implements a flag that can be treated as a boolean (--graph)
// or a string (--graph=output.svg).
type graphValue struct {
	opts *runOpts
}

var _ pflag.Value = &graphValue{}

func (d *graphValue) String() string {
	if d.opts.graphDot {
		return _graphText
	}
	return d.opts.graphFile
}

func (d *graphValue) Set(value string) error {
	if value == _graphNoValue {
		// this case matches the NoOptDefValue, which is used when the flag
		// is passed, but does not have a value (i.e. boolean flag)
		d.opts.graphDot = true
	} else if value == _graphTextValue {
		// "true" is equivalent to just setting the boolean flag
		d.opts.graphDot = true
	} else {
		d.opts.graphDot = false
		d.opts.graphFile = value
	}
	return nil
}

// Type implements Value.Type, and in this case is used to
// show the alias in the usage test.
func (d *graphValue) Type() string {
	return ""
}

// dry run custom flag
const (
	_dryRunText      = "dry run"
	_dryRunJSONText  = "json"
	_dryRunJSONValue = "json"
	_dryRunNoValue   = "text|json"
	_dryRunTextValue = "text"
)

// dryRunValue implements a flag that can be treated as a boolean (--dry-run)
// or a string (--dry-run=json).
type dryRunValue struct {
	opts *runOpts
}

var _ pflag.Value = &dryRunValue{}

func (d *dryRunValue) String() string {
	if d.opts.dryRunJSON {
		return _dryRunJSONText
	} else if d.opts.dryRun {
		return _dryRunText
	}
	return ""
}

func (d *dryRunValue) Set(value string) error {
	if value == _dryRunJSONValue {
		d.opts.dryRun = true
		d.opts.dryRunJSON = true
	} else if value == _dryRunNoValue {
		// this case matches the NoOptDefValue, which is used when the flag
		// is passed, but does not have a value (i.e. boolean flag)
		d.opts.dryRun = true
	} else if value == _dryRunTextValue {
		// "text" is equivalent to just setting the boolean flag
		d.opts.dryRun = true
	} else {
		return fmt.Errorf("invalid dry-run mode: %v", value)
	}
	return nil
}

// Type implements Value.Type, and in this case is used to
// show the alias in the usage test.
func (d *dryRunValue) Type() string {
	return "/ dry "
}

func validateTasks(pipeline fs.Pipeline, tasks []string) error {
	for _, task := range tasks {
		if !pipeline.HasTask(task) {
			return fmt.Errorf("task `%v` not found in turbo `pipeline` in \"turbo.json\". Are you sure you added it?", task)
		}
	}
	return nil
}
