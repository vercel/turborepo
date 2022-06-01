package run

import (
	gocontext "context"
	"encoding/json"
	"flag"
	"fmt"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"sync"
	"text/tabwriter"
	"time"

	"github.com/vercel/turborepo/cli/internal/analytics"
	"github.com/vercel/turborepo/cli/internal/cache"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/context"
	"github.com/vercel/turborepo/cli/internal/core"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/logstreamer"
	"github.com/vercel/turborepo/cli/internal/nodes"
	"github.com/vercel/turborepo/cli/internal/packagemanager"
	"github.com/vercel/turborepo/cli/internal/process"
	"github.com/vercel/turborepo/cli/internal/runcache"
	"github.com/vercel/turborepo/cli/internal/scm"
	"github.com/vercel/turborepo/cli/internal/scope"
	"github.com/vercel/turborepo/cli/internal/taskhash"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"
	"github.com/vercel/turborepo/cli/internal/util/browser"

	"github.com/pyr-sh/dag"

	"github.com/fatih/color"
	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
	"github.com/pkg/errors"
)

// RunCommand is a Command implementation that tells Turbo to run a task
type RunCommand struct {
	Config    *config.Config
	Ui        *cli.ColoredUi
	Processes *process.Manager
}

// completeGraph represents the common state inferred from the filesystem and pipeline.
// It is not intended to include information specific to a particular run.
type completeGraph struct {
	TopologicalGraph dag.AcyclicGraph
	Pipeline         fs.Pipeline
	PackageInfos     map[interface{}]*fs.PackageJSON
	GlobalHash       string
	RootNode         string
}

// runSpec contains the run-specific configuration elements that come from a particular
// invocation of turbo.
type runSpec struct {
	Targets      []string
	FilteredPkgs util.Set
	Opts         *Opts
}

func (rs *runSpec) ArgsForTask(task string) []string {
	passThroughArgs := make([]string, 0, len(rs.Opts.runOpts.passThroughArgs))
	for _, target := range rs.Targets {
		if target == task {
			passThroughArgs = append(passThroughArgs, rs.Opts.runOpts.passThroughArgs...)
		}
	}
	return passThroughArgs
}

// Synopsis of run command
func (c *RunCommand) Synopsis() string {
	return "Run a task"
}

// Help returns information about the `run` command
func (c *RunCommand) Help() string {
	helpText := strings.TrimSpace(`
Usage: turbo run <task> [options] [-- <args passed to tasks>]

    Run tasks across projects in your monorepo.

    By default, turbo executes tasks in topological order (i.e.
    dependencies first) and then caches the results. Re-running commands for
    tasks already in the cache will skip re-execution and immediately move
    artifacts from the cache into the correct output folders (as if the task
    occurred again).

    Arguments passed after '--' will be passed through to the named tasks.

Options:
  --help                 Show this message.
  --scope                Specify package(s) to act as entry points for task
                         execution. Supports globs.
  --cache-dir            Specify local filesystem cache directory.
                         (default "./node_modules/.cache/turbo")
  --concurrency          Limit the concurrency of task execution. Use 1 for
                         serial (i.e. one-at-a-time) execution. (default 10)
  --continue             Continue execution even if a task exits with an error
                         or non-zero exit code. The default behavior is to bail
                         immediately. (default false)
  --filter="<selector>"  Use the given selector to specify package(s) to act as
                         entry points. The syntax mirror's pnpm's syntax, and
                         additional documentation and examples can be found in
                         turbo's documentation https://turborepo.org/docs/reference/command-line-reference#--filter
                         --filter can be specified multiple times. Packages that
                         match any filter will be included.
  --force                Ignore the existing cache (to force execution).
                         (default false)
  --graph                Generate a Dot graph of the task execution.
  --global-deps          Specify glob of global filesystem dependencies to
                         be hashed. Useful for .env and files in the root
                         directory. Can be specified multiple times.
  --since                Limit/Set scope to changed packages since a
                         mergebase. This uses the git diff ${target_branch}...
                         mechanism to identify which packages have changed.
  --team                 The slug or team ID of the remote cache team.
  --token                A bearer token for remote caching. You can also set
                         the value of the current token by setting an
                         environment variable named TURBO_TOKEN.
  --ignore               Files to ignore when calculating changed files
                         (i.e. --since). Supports globs.
  --profile              File to write turbo's performance profile output into.
                         You can load the file up in chrome://tracing to see
                         which parts of your build were slow.
  --parallel             Execute all tasks in parallel. (default false)
  --include-dependencies Include the dependencies of tasks in execution.
                         (default false)
  --no-deps              Exclude dependent task consumers from execution.
                         (default false)
  --no-cache             Avoid saving task results to the cache. Useful for
                         development/watch tasks. (default false)
  --output-logs          Set type of process output logging. Use full to show
                         all output. Use hash-only to show only turbo-computed
                         task hashes. Use new-only to show only new output with
                         only hashes for cached tasks. Use none to hide process
                         output. (default full)
  --dry/--dry-run[=json] List the packages in scope and the tasks that would be run,
                         but don't actually run them. Passing --dry=json or
                         --dry-run=json will render the output in JSON format.
  --remote-only		     Ignore the local filesystem cache for all tasks. Only
                         allow reading and caching artifacts using the remote cache.
`)
	return strings.TrimSpace(helpText)
}

// Run executes tasks in the monorepo
func (c *RunCommand) Run(args []string) int {
	startAt := time.Now()
	log.SetFlags(0)
	flags := flag.NewFlagSet("run", flag.ContinueOnError)
	flags.Usage = func() { c.Config.Logger.Info(c.Help()) }
	if err := flags.Parse(args); err != nil {
		return 1
	}

	opts, err := parseRunArgs(args, c.Config, c.Ui)
	if err != nil {
		c.logError(c.Config.Logger, "", err)
		return 1
	}

	ctx, err := context.New(context.WithGraph(c.Config, opts.cacheOpts.Dir))
	if err != nil {
		c.logError(c.Config.Logger, "", err)
		return 1
	}

	if err := util.ValidateGraph(&ctx.TopologicalGraph); err != nil {
		c.logError(c.Config.Logger, "Invalid package dependency graph:\n%v", err)
		return 1
	}

	pipeline := c.Config.TurboJSON.Pipeline
	targets, err := getTargetsFromArguments(args, pipeline)
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("failed to resolve targets: %w", err))
		return 1
	}

	scmInstance, err := scm.FromInRepo(c.Config.Cwd.ToStringDuringMigration())
	if err != nil {
		if errors.Is(err, scm.ErrFallback) {
			c.logWarning(c.Config.Logger, "", err)
		} else {
			c.logError(c.Config.Logger, "", fmt.Errorf("failed to create SCM: %w", err))
			return 1
		}
	}
	filteredPkgs, isAllPackages, err := scope.ResolvePackages(&opts.scopeOpts, c.Config.Cwd.ToStringDuringMigration(), scmInstance, ctx, c.Ui, c.Config.Logger)
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("failed to resolve packages to run: %v", err))
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
	c.Config.Logger.Debug("global hash", "value", ctx.GlobalHash)
	c.Config.Logger.Debug("local cache folder", "path", opts.cacheOpts.Dir)

	// TODO: consolidate some of these arguments
	g := &completeGraph{
		TopologicalGraph: ctx.TopologicalGraph,
		Pipeline:         pipeline,
		PackageInfos:     ctx.PackageInfos,
		GlobalHash:       ctx.GlobalHash,
		RootNode:         ctx.RootNode,
	}
	rs := &runSpec{
		Targets:      targets,
		FilteredPkgs: filteredPkgs,
		Opts:         opts,
	}
	packageManager := ctx.PackageManager
	return c.runOperation(g, rs, packageManager, startAt)
}

func (c *RunCommand) runOperation(g *completeGraph, rs *runSpec, packageManager *packagemanager.PackageManager, startAt time.Time) int {
	vertexSet := make(util.Set)
	for _, v := range g.TopologicalGraph.Vertices() {
		vertexSet.Add(v)
	}

	engine, err := buildTaskGraph(&g.TopologicalGraph, g.Pipeline, rs)
	if err != nil {
		c.Ui.Error(fmt.Sprintf("Error preparing engine: %s", err))
		return 1
	}
	hashTracker := taskhash.NewTracker(g.RootNode, g.GlobalHash, g.Pipeline, g.PackageInfos)
	err = hashTracker.CalculateFileHashes(engine.TaskGraph.Vertices(), rs.Opts.runOpts.concurrency, c.Config.Cwd)
	if err != nil {
		c.Ui.Error(fmt.Sprintf("Error hashing package files: %s", err))
		return 1
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
		engine, err = buildTaskGraph(&g.TopologicalGraph, g.Pipeline, rs)
		if err != nil {
			c.Ui.Error(fmt.Sprintf("Error preparing engine: %s", err))
			return 1
		}
	}

	exitCode := 0
	if rs.Opts.runOpts.dotGraph != "" {
		err := c.generateDotGraph(engine.TaskGraph, c.Config.Cwd.Join(rs.Opts.runOpts.dotGraph))
		if err != nil {
			c.logError(c.Config.Logger, "", err)
			return 1
		}
	} else if rs.Opts.runOpts.dryRun {
		tasksRun, err := c.executeDryRun(engine, g, hashTracker, rs)
		if err != nil {
			c.logError(c.Config.Logger, "", err)
			return 1
		}
		packagesInScope := rs.FilteredPkgs.UnsafeListOfStrings()
		sort.Strings(packagesInScope)
		if rs.Opts.runOpts.dryRunJSON {
			dryRun := &struct {
				Packages []string     `json:"packages"`
				Tasks    []hashedTask `json:"tasks"`
			}{
				Packages: packagesInScope,
				Tasks:    tasksRun,
			}
			bytes, err := json.MarshalIndent(dryRun, "", "  ")
			if err != nil {
				c.logError(c.Config.Logger, "", errors.Wrap(err, "failed to render to JSON"))
				return 1
			}
			c.Ui.Output(string(bytes))
		} else {
			c.Ui.Output("")
			c.Ui.Info(util.Sprintf("${CYAN}${BOLD}Packages in Scope${RESET}"))
			p := tabwriter.NewWriter(os.Stdout, 0, 0, 1, ' ', 0)
			fmt.Fprintln(p, "Name\tPath\t")
			for _, pkg := range packagesInScope {
				fmt.Fprintf(p, "%s\t%s\t\n", pkg, g.PackageInfos[pkg].Dir)
			}
			p.Flush()

			c.Ui.Output("")
			c.Ui.Info(util.Sprintf("${CYAN}${BOLD}Tasks to Run${RESET}"))

			for _, task := range tasksRun {
				c.Ui.Info(util.Sprintf("${BOLD}%s${RESET}", task.TaskID))
				w := tabwriter.NewWriter(os.Stdout, 0, 0, 1, ' ', 0)
				fmt.Fprintln(w, util.Sprintf("  ${GREY}Task\t=\t%s\t${RESET}", task.Task))
				fmt.Fprintln(w, util.Sprintf("  ${GREY}Package\t=\t%s\t${RESET}", task.Package))
				fmt.Fprintln(w, util.Sprintf("  ${GREY}Hash\t=\t%s\t${RESET}", task.Hash))
				fmt.Fprintln(w, util.Sprintf("  ${GREY}Directory\t=\t%s\t${RESET}", task.Dir))
				fmt.Fprintln(w, util.Sprintf("  ${GREY}Command\t=\t%s\t${RESET}", task.Command))
				fmt.Fprintln(w, util.Sprintf("  ${GREY}Outputs\t=\t%s\t${RESET}", strings.Join(task.Outputs, ", ")))
				fmt.Fprintln(w, util.Sprintf("  ${GREY}Log File\t=\t%s\t${RESET}", task.LogFile))
				fmt.Fprintln(w, util.Sprintf("  ${GREY}Dependencies\t=\t%s\t${RESET}", strings.Join(task.Dependencies, ", ")))
				fmt.Fprintln(w, util.Sprintf("  ${GREY}Dependendents\t=\t%s\t${RESET}", strings.Join(task.Dependents, ", ")))
				w.Flush()
			}

		}
	} else {
		packagesInScope := rs.FilteredPkgs.UnsafeListOfStrings()
		sort.Strings(packagesInScope)
		c.Ui.Output(fmt.Sprintf(ui.Dim("• Packages in scope: %v"), strings.Join(packagesInScope, ", ")))
		c.Ui.Output(fmt.Sprintf("%s %s %s", ui.Dim("• Running"), ui.Dim(ui.Bold(strings.Join(rs.Targets, ", "))), ui.Dim(fmt.Sprintf("in %v packages", rs.FilteredPkgs.Len()))))
		exitCode = c.executeTasks(g, rs, engine, packageManager, hashTracker, startAt)
	}

	return exitCode
}

func buildTaskGraph(topoGraph *dag.AcyclicGraph, pipeline fs.Pipeline, rs *runSpec) (*core.Scheduler, error) {
	engine := core.NewScheduler(topoGraph)
	for taskName, taskDefinition := range pipeline {
		topoDeps := make(util.Set)
		deps := make(util.Set)
		isPackageTask := util.IsPackageTask(taskName)
		for _, dependency := range taskDefinition.TaskDependencies {
			if isPackageTask && util.IsPackageTask(dependency) {
				err := engine.AddDep(dependency, taskName)
				if err != nil {
					return nil, err
				}
			} else {
				deps.Add(dependency)
			}
		}
		for _, dependency := range taskDefinition.TopologicalDependencies {
			topoDeps.Add(dependency)
		}
		engine.AddTask(&core.Task{
			Name:     taskName,
			TopoDeps: topoDeps,
			Deps:     deps,
		})
	}

	if err := engine.Prepare(&core.SchedulerExecutionOptions{
		Packages:  rs.FilteredPkgs.UnsafeListOfStrings(),
		TaskNames: rs.Targets,
		TasksOnly: rs.Opts.runOpts.only,
	}); err != nil {
		return nil, err
	}

	if err := util.ValidateGraph(engine.TaskGraph); err != nil {
		return nil, fmt.Errorf("Invalid task dependency graph:\n%v", err)
	}

	return engine, nil
}

// Opts holds the current run operations configuration
type Opts struct {
	runOpts      runOpts
	cacheOpts    cache.Opts
	runcacheOpts runcache.Opts
	scopeOpts    scope.Opts
}

// runOpts holds the options that control the execution of a turbo run
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
	only       bool
	dryRun     bool
	dryRunJSON bool
}

func getDefaultOptions(config *config.Config) *Opts {
	return &Opts{
		runOpts: runOpts{
			concurrency: 10,
		},
		cacheOpts: cache.Opts{
			Dir:     cache.DefaultLocation(config.Cwd),
			Workers: config.Cache.Workers,
		},
		scopeOpts: scope.Opts{},
	}
}

func parseRunArgs(args []string, config *config.Config, output cli.Ui) (*Opts, error) {
	opts := getDefaultOptions(config)

	if len(args) == 0 {
		return nil, errors.Errorf("At least one task must be specified.")
	}

	var unresolvedCacheFolder string

	if os.Getenv("TURBO_FORCE") == "true" {
		opts.runcacheOpts.SkipReads = true
	}

	if os.Getenv("TURBO_REMOTE_ONLY") == "true" {
		opts.cacheOpts.SkipFilesystem = true
	}

	for argIndex, arg := range args {
		if arg == "--" {
			opts.runOpts.passThroughArgs = args[argIndex+1:]
			break
		} else if strings.HasPrefix(arg, "--") {
			switch {
			case strings.HasPrefix(arg, "--filter="):
				filterPattern := arg[len("--filter="):]
				if filterPattern != "" {
					opts.scopeOpts.FilterPatterns = append(opts.scopeOpts.FilterPatterns, filterPattern)
				}
			case strings.HasPrefix(arg, "--since="):
				if len(arg[len("--since="):]) > 0 {
					opts.scopeOpts.LegacyFilter.Since = arg[len("--since="):]
				}
			case strings.HasPrefix(arg, "--scope="):
				if len(arg[len("--scope="):]) > 0 {
					opts.scopeOpts.LegacyFilter.Entrypoints = append(opts.scopeOpts.LegacyFilter.Entrypoints, arg[len("--scope="):])
				}
			case strings.HasPrefix(arg, "--ignore="):
				if len(arg[len("--ignore="):]) > 0 {
					opts.scopeOpts.IgnorePatterns = append(opts.scopeOpts.IgnorePatterns, arg[len("--ignore="):])
				}
			case strings.HasPrefix(arg, "--global-deps="):
				if len(arg[len("--global-deps="):]) > 0 {
					opts.scopeOpts.GlobalDepPatterns = append(opts.scopeOpts.GlobalDepPatterns, arg[len("--global-deps="):])
				}
			case strings.HasPrefix(arg, "--parallel"):
				opts.runOpts.parallel = true
			case strings.HasPrefix(arg, "--profile="): // this one must com before the next
				if len(arg[len("--profile="):]) > 0 {
					opts.runOpts.profile = arg[len("--profile="):]
				}
			case strings.HasPrefix(arg, "--profile"):
				opts.runOpts.profile = fmt.Sprintf("%v-profile.json", time.Now().UnixNano())

			case strings.HasPrefix(arg, "--no-deps"):
				opts.scopeOpts.LegacyFilter.SkipDependents = true
			case strings.HasPrefix(arg, "--no-cache"):
				opts.runcacheOpts.SkipWrites = true
			case strings.HasPrefix(arg, "--cacheFolder"):
				output.Warn("[WARNING] The --cacheFolder flag has been deprecated and will be removed in future versions of turbo. Please use `--cache-dir` instead")
				unresolvedCacheFolder = arg[len("--cacheFolder="):]
			case strings.HasPrefix(arg, "--cache-dir"):
				unresolvedCacheFolder = arg[len("--cache-dir="):]
			case strings.HasPrefix(arg, "--continue"):
				opts.runOpts.continueOnError = true
			case strings.HasPrefix(arg, "--force"):
				opts.runcacheOpts.SkipReads = true
			case strings.HasPrefix(arg, "--stream"):
				output.Warn("[WARNING] The --stream flag is unnecesary and has been deprecated. It will be removed in future versions of turbo.")
			case strings.HasPrefix(arg, "--graph="): // this one must com before the next
				if len(arg[len("--graph="):]) > 0 {
					opts.runOpts.dotGraph = arg[len("--graph="):]
				}
			case strings.HasPrefix(arg, "--graph"):
				opts.runOpts.dotGraph = fmt.Sprintf("graph-%v.jpg", time.Now().UnixNano())
			case strings.HasPrefix(arg, "--serial"):
				output.Warn("[WARNING] The --serial flag has been deprecated and will be removed in future versions of turbo. Please use `--concurrency=1` instead")
				opts.runOpts.concurrency = 1
			case strings.HasPrefix(arg, "--concurrency"):
				concurrencyRaw := arg[len("--concurrency="):]
				if concurrency, err := util.ParseConcurrency(concurrencyRaw); err != nil {
					return nil, err
				} else {
					opts.runOpts.concurrency = concurrency
				}
			case strings.HasPrefix(arg, "--includeDependencies"):
				output.Warn("[WARNING] The --includeDependencies flag has renamed to --include-dependencies for consistency. Please use `--include-dependencies` instead")
				opts.scopeOpts.LegacyFilter.IncludeDependencies = true
			case strings.HasPrefix(arg, "--include-dependencies"):
				opts.scopeOpts.LegacyFilter.IncludeDependencies = true
			case strings.HasPrefix(arg, "--only"):
				opts.runOpts.only = true
			case strings.HasPrefix(arg, "--output-logs="):
				outputLogsMode := arg[len("--output-logs="):]
				switch outputLogsMode {
				case "full":
					opts.runcacheOpts.CacheMissLogsMode = runcache.FullLogs
					opts.runcacheOpts.CacheHitLogsMode = runcache.FullLogs
				case "none":
					opts.runcacheOpts.CacheMissLogsMode = runcache.NoLogs
					opts.runcacheOpts.CacheHitLogsMode = runcache.NoLogs
				case "hash-only":
					opts.runcacheOpts.CacheMissLogsMode = runcache.HashLogs
					opts.runcacheOpts.CacheHitLogsMode = runcache.HashLogs
				case "new-only":
					opts.runcacheOpts.CacheMissLogsMode = runcache.FullLogs
					opts.runcacheOpts.CacheHitLogsMode = runcache.HashLogs
				default:
					output.Warn(fmt.Sprintf("[WARNING] unknown value %v for --output-logs CLI flag. Falling back to full", outputLogsMode))
				}
			case strings.HasPrefix(arg, "--dry-run"):
				opts.runOpts.dryRun = true
				if strings.HasPrefix(arg, "--dry-run=json") {
					opts.runOpts.dryRunJSON = true
				}
			case strings.HasPrefix(arg, "--dry"):
				opts.runOpts.dryRun = true
				if strings.HasPrefix(arg, "--dry=json") {
					opts.runOpts.dryRunJSON = true
				}
			case strings.HasPrefix(arg, "--remote-only"):
				opts.cacheOpts.SkipFilesystem = true
			case strings.HasPrefix(arg, "--team"):
			case strings.HasPrefix(arg, "--token"):
			case strings.HasPrefix(arg, "--preflight"):
			case strings.HasPrefix(arg, "--api"):
			case strings.HasPrefix(arg, "--url"):
			case strings.HasPrefix(arg, "--trace"):
			case strings.HasPrefix(arg, "--cpuprofile"):
			case strings.HasPrefix(arg, "--heap"):
			case strings.HasPrefix(arg, "--no-gc"):
			case strings.HasPrefix(arg, "--cwd="):
			default:
				return nil, errors.New(fmt.Sprintf("unknown flag: %v", arg))
			}
		}
	}

	// We can only set this cache folder after we know actual cwd
	if unresolvedCacheFolder != "" {
		opts.cacheOpts.Dir = fs.ResolveUnknownPath(config.Cwd, unresolvedCacheFolder)
	}
	if !config.IsLoggedIn() {
		opts.cacheOpts.SkipRemote = true
	}

	return opts, nil
}

// logError logs an error and outputs it to the UI.
func (c *RunCommand) logError(log hclog.Logger, prefix string, err error) {
	log.Error(prefix, "error", err)

	if prefix != "" {
		prefix += ": "
	}

	c.Ui.Error(fmt.Sprintf("%s%s%s", ui.ERROR_PREFIX, prefix, color.RedString(" %v", err)))
}

// logError logs an error and outputs it to the UI.
func (c *RunCommand) logWarning(log hclog.Logger, prefix string, err error) {
	log.Warn(prefix, "warning", err)

	if prefix != "" {
		prefix = " " + prefix + ": "
	}

	c.Ui.Error(fmt.Sprintf("%s%s%s", ui.WARNING_PREFIX, prefix, color.YellowString(" %v", err)))
}

func hasGraphViz() bool {
	err := exec.Command("dot", "-v").Run()
	return err == nil
}

func (c *RunCommand) executeTasks(g *completeGraph, rs *runSpec, engine *core.Scheduler, packageManager *packagemanager.PackageManager, hashes *taskhash.Tracker, startAt time.Time) int {
	goctx := gocontext.Background()
	var analyticsSink analytics.Sink
	if c.Config.IsLoggedIn() {
		analyticsSink = c.Config.ApiClient
	} else {
		analyticsSink = analytics.NullSink
	}
	analyticsClient := analytics.NewClient(goctx, analyticsSink, c.Config.Logger.Named("analytics"))
	defer analyticsClient.CloseWithTimeout(50 * time.Millisecond)
	// Theoretically this is overkill, but bias towards not spamming the console
	once := &sync.Once{}
	turboCache, err := cache.New(rs.Opts.cacheOpts, c.Config, analyticsClient, func(_cache cache.Cache, err error) {
		// Currently the HTTP Cache is the only one that can be disabled.
		// With a cache system refactor, we might consider giving names to the caches so
		// we can accurately report them here.
		once.Do(func() {
			c.logWarning(c.Config.Logger, "Remote Caching is unavailable", err)
		})
	})
	if err != nil {
		if errors.Is(err, cache.ErrNoCachesEnabled) {
			c.logWarning(c.Config.Logger, "No caches are enabled. You can try \"turbo login\", \"turbo link\", or ensuring you are not passing --remote-only to enable caching", nil)
		} else {
			c.logError(c.Config.Logger, "Failed to set up caching", err)
			return 1
		}
	}
	defer turboCache.Shutdown()
	runState := NewRunState(startAt, rs.Opts.runOpts.profile)
	runCache := runcache.New(turboCache, c.Config.Cwd, rs.Opts.runcacheOpts)
	ec := &execContext{
		colorCache:     NewColorCache(),
		runState:       runState,
		rs:             rs,
		ui:             &cli.ConcurrentUi{Ui: c.Ui},
		turboCache:     turboCache,
		runCache:       runCache,
		logger:         c.Config.Logger,
		packageManager: packageManager,
		processes:      c.Processes,
		taskHashes:     hashes,
	}

	// run the thing
	errs := engine.Execute(g.getPackageTaskVisitor(func(pt *nodes.PackageTask) error {
		deps := engine.TaskGraph.DownEdges(pt.TaskID)
		return ec.exec(pt, deps)
	}), core.ExecOpts{
		Parallel:    rs.Opts.runOpts.parallel,
		Concurrency: rs.Opts.runOpts.concurrency,
	})

	// Track if we saw any child with a non-zero exit code
	exitCode := 0
	exitCodeErr := &process.ChildExit{}
	for _, err := range errs {
		if errors.As(err, &exitCodeErr) {
			if exitCodeErr.ExitCode > exitCode {
				exitCode = exitCodeErr.ExitCode
			}
		} else if exitCode == 0 {
			// We hit some error, it shouldn't be exit code 0
			exitCode = 1
		}
		c.Ui.Error(err.Error())
	}

	if err := runState.Close(c.Ui, rs.Opts.runOpts.profile); err != nil {
		c.Ui.Error(fmt.Sprintf("Error with profiler: %s", err.Error()))
		return 1
	}
	return exitCode
}

type hashedTask struct {
	TaskID       string   `json:"taskId"`
	Task         string   `json:"task"`
	Package      string   `json:"package"`
	Hash         string   `json:"hash"`
	Command      string   `json:"command"`
	Outputs      []string `json:"outputs"`
	LogFile      string   `json:"logFile"`
	Dir          string   `json:"directory"`
	Dependencies []string `json:"dependencies"`
	Dependents   []string `json:"dependents"`
}

func (c *RunCommand) executeDryRun(engine *core.Scheduler, g *completeGraph, taskHashes *taskhash.Tracker, rs *runSpec) ([]hashedTask, error) {
	taskIDs := []hashedTask{}
	errs := engine.Execute(g.getPackageTaskVisitor(func(pt *nodes.PackageTask) error {
		passThroughArgs := rs.ArgsForTask(pt.Task)
		deps := engine.TaskGraph.DownEdges(pt.TaskID)
		hash, err := taskHashes.CalculateTaskHash(pt, deps, passThroughArgs)
		if err != nil {
			return err
		}
		command, ok := pt.Command()
		if !ok {
			command = "<NONEXISTENT>"
		}
		isRootTask := pt.PackageName == util.RootPkgName
		if isRootTask && commandLooksLikeTurbo(command) {
			return fmt.Errorf("root task %v (%v) looks like it invokes turbo and might cause a loop", pt.Task, command)
		}
		ancestors, err := engine.TaskGraph.Ancestors(pt.TaskID)
		if err != nil {
			return err
		}
		stringAncestors := []string{}
		for _, dep := range ancestors {
			// Don't leak out internal ROOT_NODE_NAME nodes, which are just placeholders
			if !strings.Contains(dep.(string), core.ROOT_NODE_NAME) {
				stringAncestors = append(stringAncestors, dep.(string))
			}
		}
		descendents, err := engine.TaskGraph.Descendents(pt.TaskID)
		if err != nil {
			return err
		}
		stringDescendents := []string{}
		for _, dep := range descendents {
			// Don't leak out internal ROOT_NODE_NAME nodes, which are just placeholders
			if !strings.Contains(dep.(string), core.ROOT_NODE_NAME) {
				stringDescendents = append(stringDescendents, dep.(string))
			}
		}
		sort.Strings(stringDescendents)

		taskIDs = append(taskIDs, hashedTask{
			TaskID:       pt.TaskID,
			Task:         pt.Task,
			Package:      pt.PackageName,
			Hash:         hash,
			Command:      command,
			Dir:          pt.Pkg.Dir,
			Outputs:      pt.TaskDefinition.Outputs,
			LogFile:      pt.RepoRelativeLogFile(),
			Dependencies: stringAncestors,
			Dependents:   stringDescendents,
		})
		return nil
	}), core.ExecOpts{
		Concurrency: 1,
		Parallel:    false,
	})
	if len(errs) > 0 {
		for _, err := range errs {
			c.Ui.Error(err.Error())
		}
		return nil, errors.New("errors occurred during dry-run graph traversal")
	}
	return taskIDs, nil
}

var _isTurbo = regexp.MustCompile(fmt.Sprintf("(?:^|%v|\\s)turbo(?:$|\\s)", regexp.QuoteMeta(string(filepath.Separator))))

func commandLooksLikeTurbo(command string) bool {
	return _isTurbo.MatchString(command)
}

// GetTargetsFromArguments returns a list of targets from the arguments and Turbo config.
// Return targets are always unique sorted alphabetically.
func getTargetsFromArguments(arguments []string, pipeline fs.Pipeline) ([]string, error) {
	targets := make(util.Set)
	for _, arg := range arguments {
		if arg == "--" {
			break
		}
		if !strings.HasPrefix(arg, "-") {
			task := arg
			if pipeline.HasTask(task) {
				targets.Add(task)
			} else {
				return nil, fmt.Errorf("task `%v` not found in turbo `pipeline` in \"turbo.json\". Are you sure you added it?", task)
			}
		}
	}
	stringTargets := targets.UnsafeListOfStrings()
	sort.Strings(stringTargets)
	return stringTargets, nil
}

type execContext struct {
	colorCache     *ColorCache
	runState       *RunState
	rs             *runSpec
	ui             cli.Ui
	runCache       *runcache.RunCache
	turboCache     cache.Cache
	logger         hclog.Logger
	packageManager *packagemanager.PackageManager
	processes      *process.Manager
	taskHashes     *taskhash.Tracker
}

func (e *execContext) logError(log hclog.Logger, prefix string, err error) {
	e.logger.Error(prefix, "error", err)

	if prefix != "" {
		prefix += ": "
	}

	e.ui.Error(fmt.Sprintf("%s%s%s", ui.ERROR_PREFIX, prefix, color.RedString(" %v", err)))
}

func (e *execContext) exec(pt *nodes.PackageTask, deps dag.Set) error {
	cmdTime := time.Now()

	targetLogger := e.logger.Named(pt.OutputPrefix())
	targetLogger.Debug("start")

	// Setup tracer
	tracer := e.runState.Run(pt.TaskID)

	// Create a logger
	pref := e.colorCache.PrefixColor(pt.PackageName)
	actualPrefix := pref("%s: ", pt.OutputPrefix())
	targetUi := &cli.PrefixedUi{
		Ui:           e.ui,
		OutputPrefix: actualPrefix,
		InfoPrefix:   actualPrefix,
		ErrorPrefix:  actualPrefix,
		WarnPrefix:   actualPrefix,
	}

	passThroughArgs := e.rs.ArgsForTask(pt.Task)
	hash, err := e.taskHashes.CalculateTaskHash(pt, deps, passThroughArgs)
	e.logger.Debug("task hash", "value", hash)
	if err != nil {
		e.ui.Error(fmt.Sprintf("Hashing error: %v", err))
		// @TODO probably should abort fatally???
	}
	// TODO(gsoltis): if/when we fix https://github.com/vercel/turborepo/issues/937
	// the following block should never get hit. In the meantime, keep it after hashing
	// so that downstream tasks can count on the hash existing
	//
	// bail if the script doesn't exist
	if _, ok := pt.Command(); !ok {
		targetLogger.Debug("no task in package, skipping")
		targetLogger.Debug("done", "status", "skipped", "duration", time.Since(cmdTime))
		return nil
	}
	// Cache ---------------------------------------------
	taskCache := e.runCache.TaskCache(pt, hash)
	hit, err := taskCache.RestoreOutputs(targetUi, targetLogger)
	if err != nil {
		targetUi.Error(fmt.Sprintf("error fetching from cache: %s", err))
	} else if hit {
		tracer(TargetCached, nil)
		return nil
	}
	// Setup command execution
	argsactual := append([]string{"run"}, pt.Task)
	argsactual = append(argsactual, passThroughArgs...)

	cmd := exec.Command(e.packageManager.Command, argsactual...)
	cmd.Dir = pt.Pkg.Dir
	envs := fmt.Sprintf("TURBO_HASH=%v", hash)
	cmd.Env = append(os.Environ(), envs)

	// Setup stdout/stderr
	// If we are not caching anything, then we don't need to write logs to disk
	// be careful about this conditional given the default of cache = true
	writer, err := taskCache.OutputWriter()
	if err != nil {
		tracer(TargetBuildFailed, err)
		e.logError(targetLogger, actualPrefix, err)
		if !e.rs.Opts.runOpts.continueOnError {
			os.Exit(1)
		}
	}
	logger := log.New(writer, "", 0)
	// Setup a streamer that we'll pipe cmd.Stdout to
	logStreamerOut := logstreamer.NewLogstreamer(logger, actualPrefix, false)
	// Setup a streamer that we'll pipe cmd.Stderr to.
	logStreamerErr := logstreamer.NewLogstreamer(logger, actualPrefix, false)
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
	if err := e.processes.Exec(cmd); err != nil {
		// close off our outputs. We errored, so we mostly don't care if we fail to close
		_ = closeOutputs()
		// if we already know we're in the process of exiting,
		// we don't need to record an error to that effect.
		if errors.Is(err, process.ErrClosing) {
			return nil
		}
		tracer(TargetBuildFailed, err)
		targetLogger.Error("Error: command finished with error: %w", err)
		if !e.rs.Opts.runOpts.continueOnError {
			targetUi.Error(fmt.Sprintf("ERROR: command finished with error: %s", err))
			e.processes.Close()
		} else {
			targetUi.Warn("command finished with error, but continuing...")
		}
		return err
	}

	duration := time.Since(cmdTime)
	// Close off our outputs and cache them
	if err := closeOutputs(); err != nil {
		e.logError(targetLogger, "", err)
	} else {
		if err = taskCache.SaveOutputs(targetLogger, targetUi, int(duration.Milliseconds())); err != nil {
			e.logError(targetLogger, "", fmt.Errorf("error caching output: %w", err))
		}
	}

	// Clean up tracing
	tracer(TargetBuilt, nil)
	targetLogger.Debug("done", "status", "complete", "duration", duration)
	return nil
}

func (c *RunCommand) generateDotGraph(taskGraph *dag.AcyclicGraph, outputFilename fs.AbsolutePath) error {
	graphString := string(taskGraph.Dot(&dag.DotOpts{
		Verbose:    true,
		DrawCycles: true,
	}))
	ext := outputFilename.Ext()
	if ext == ".html" {
		f, err := outputFilename.Create()
		if err != nil {
			return fmt.Errorf("error writing graph: %w", err)
		}
		defer f.Close()
		f.WriteString(`<!DOCTYPE html>
    <html>
    <head>
      <meta charset="utf-8">
      <title>Graph</title>
    </head>
    <body>
      <script src="https://cdn.jsdelivr.net/npm/viz.js@2.1.2-pre.1/viz.js"></script>
      <script src="https://cdn.jsdelivr.net/npm/viz.js@2.1.2-pre.1/full.render.js"></script>
      <script>`)
		f.WriteString("const s = `" + graphString + "`.replace(/\\_\\_\\_ROOT\\_\\_\\_/g, \"Root\").replace(/\\[root\\]/g, \"\");new Viz().renderSVGElement(s).then(el => document.body.appendChild(el)).catch(e => console.error(e));")
		f.WriteString(`
    </script>
  </body>
  </html>`)
		c.Ui.Output("")
		c.Ui.Output(fmt.Sprintf("✔ Generated task graph in %s", ui.Bold(outputFilename.ToString())))
		if ui.IsTTY {
			if err := browser.OpenBrowser(outputFilename.ToString()); err != nil {
				c.Ui.Warn(color.New(color.FgYellow, color.Bold, color.ReverseVideo).Sprintf("failed to open browser. Please navigate to file://%v", filepath.ToSlash(outputFilename.ToString())))
			}
		}
		return nil
	}
	hasDot := hasGraphViz()
	if hasDot {
		dotArgs := []string{"-T" + ext[1:], "-o", outputFilename.ToString()}
		cmd := exec.Command("dot", dotArgs...)
		cmd.Stdin = strings.NewReader(graphString)
		if err := cmd.Run(); err != nil {
			return fmt.Errorf("could not generate task graphfile %v:  %w", outputFilename, err)
		} else {
			c.Ui.Output("")
			c.Ui.Output(fmt.Sprintf("✔ Generated task graph in %s", ui.Bold(outputFilename.ToString())))
		}
	} else {
		c.Ui.Output("")
		c.Ui.Warn(color.New(color.FgYellow, color.Bold, color.ReverseVideo).Sprint(" WARNING ") + color.YellowString(" `turbo` uses Graphviz to generate an image of your\ngraph, but Graphviz isn't installed on this machine.\n\nYou can download Graphviz from https://graphviz.org/download.\n\nIn the meantime, you can use this string output with an\nonline Dot graph viewer."))
		c.Ui.Output("")
		c.Ui.Output(graphString)
	}
	return nil
}

func (g *completeGraph) getPackageTaskVisitor(visitor func(pt *nodes.PackageTask) error) func(taskID string) error {
	return func(taskID string) error {
		name, task := util.GetPackageTaskFromId(taskID)
		pkg, ok := g.PackageInfos[name]
		if !ok {
			return fmt.Errorf("cannot find package %v for task %v", name, taskID)
		}
		// first check for package-tasks
		pipeline, ok := g.Pipeline[fmt.Sprintf("%v", taskID)]
		if !ok {
			// then check for regular tasks
			altpipe, notcool := g.Pipeline[task]
			// if neither, then bail
			if !notcool && !ok {
				return nil
			}
			// override if we need to...
			pipeline = altpipe
		}
		return visitor(&nodes.PackageTask{
			TaskID:         taskID,
			Task:           task,
			PackageName:    name,
			Pkg:            pkg,
			TaskDefinition: &pipeline,
		})
	}
}
