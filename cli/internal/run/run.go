package run

import (
	gocontext "context"
	"encoding/json"
	"fmt"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"sync"
	"text/tabwriter"
	"time"

	"github.com/pyr-sh/dag"
	"github.com/spf13/cobra"
	"github.com/spf13/pflag"
	"github.com/vercel/turborepo/cli/internal/analytics"
	"github.com/vercel/turborepo/cli/internal/cache"
	"github.com/vercel/turborepo/cli/internal/cmdutil"
	"github.com/vercel/turborepo/cli/internal/colorcache"
	"github.com/vercel/turborepo/cli/internal/context"
	"github.com/vercel/turborepo/cli/internal/core"
	"github.com/vercel/turborepo/cli/internal/daemon"
	"github.com/vercel/turborepo/cli/internal/daemonclient"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/graphvisualizer"
	"github.com/vercel/turborepo/cli/internal/logstreamer"
	"github.com/vercel/turborepo/cli/internal/nodes"
	"github.com/vercel/turborepo/cli/internal/packagemanager"
	"github.com/vercel/turborepo/cli/internal/process"
	"github.com/vercel/turborepo/cli/internal/runcache"
	"github.com/vercel/turborepo/cli/internal/scm"
	"github.com/vercel/turborepo/cli/internal/scope"
	"github.com/vercel/turborepo/cli/internal/signals"
	"github.com/vercel/turborepo/cli/internal/spinner"
	"github.com/vercel/turborepo/cli/internal/taskhash"
	"github.com/vercel/turborepo/cli/internal/turbopath"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"

	"github.com/fatih/color"
	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
	"github.com/pkg/errors"
)

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
			base, err := helper.GetCmdBase(cmd.Flags())
			if err != nil {
				return err
			}
			tasks, passThroughArgs := parseTasksAndPassthroughArgs(args, flags)
			if len(tasks) == 0 {
				return errors.New("at least one task must be specified")
			}
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
		pkgDepGraph, err = context.BuildPackageGraph(r.base.RepoRoot, rootPackageJSON, r.opts.cacheOpts.ResolveCacheDir(r.base.RepoRoot))
	}
	if err != nil {
		return err
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

	scmInstance, err := scm.FromInRepo(r.base.RepoRoot.ToStringDuringMigration())
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
		r.base.Logger,
		os.Environ(),
	)
	if err != nil {
		return fmt.Errorf("failed to calculate global hash: %v", err)
	}
	r.base.Logger.Debug("global hash", "value", globalHash)
	r.base.Logger.Debug("local cache folder", "path", r.opts.cacheOpts.OverrideDir)

	// TODO: consolidate some of these arguments
	g := &completeGraph{
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
	return r.runOperation(ctx, g, rs, packageManager, startAt)
}

func (r *run) runOperation(ctx gocontext.Context, g *completeGraph, rs *runSpec, packageManager *packagemanager.PackageManager, startAt time.Time) error {
	vertexSet := make(util.Set)
	for _, v := range g.TopologicalGraph.Vertices() {
		vertexSet.Add(v)
	}

	engine, err := buildTaskGraph(&g.TopologicalGraph, g.Pipeline, rs)
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
		engine, err = buildTaskGraph(&g.TopologicalGraph, g.Pipeline, rs)
		if err != nil {
			return errors.Wrap(err, "error preparing engine")
		}
	}

	if rs.Opts.runOpts.graphFile != "" || rs.Opts.runOpts.graphDot {
		graph := engine.TaskGraph
		if r.opts.runOpts.singlePackage {
			graph = filterSinglePackageGraphForDisplay(engine.TaskGraph)
		}
		visualizer := graphvisualizer.New(r.base.RepoRoot, r.base.UI, graph)

		if rs.Opts.runOpts.graphDot {
			visualizer.RenderDotGraph()
		} else {
			err := visualizer.GenerateGraphFile(rs.Opts.runOpts.graphFile)
			if err != nil {
				return err
			}
		}
	} else if rs.Opts.runOpts.dryRun {
		tasksRun, err := r.executeDryRun(ctx, engine, g, tracker, rs)
		if err != nil {
			return err
		}
		packagesInScope := rs.FilteredPkgs.UnsafeListOfStrings()
		sort.Strings(packagesInScope)
		if rs.Opts.runOpts.dryRunJSON {
			var rendered string
			if r.opts.runOpts.singlePackage {
				rendered, err = renderDryRunSinglePackageJSON(tasksRun)
			} else {
				rendered, err = renderDryRunFullJSON(tasksRun, packagesInScope)
			}
			if err != nil {
				return err
			}
			r.base.UI.Output(rendered)
		} else {
			if err := displayDryTextRun(r.base.UI, tasksRun, packagesInScope, g.PackageInfos, r.opts.runOpts.singlePackage); err != nil {
				return err
			}
		}
	} else {
		packagesInScope := rs.FilteredPkgs.UnsafeListOfStrings()
		sort.Strings(packagesInScope)
		if r.opts.runOpts.singlePackage {
			r.base.UI.Output(fmt.Sprintf("%s %s", ui.Dim("• Running"), ui.Dim(ui.Bold(strings.Join(rs.Targets, ", ")))))
		} else {
			r.base.UI.Output(fmt.Sprintf(ui.Dim("• Packages in scope: %v"), strings.Join(packagesInScope, ", ")))
			r.base.UI.Output(fmt.Sprintf("%s %s %s", ui.Dim("• Running"), ui.Dim(ui.Bold(strings.Join(rs.Targets, ", "))), ui.Dim(fmt.Sprintf("in %v packages", rs.FilteredPkgs.Len()))))
		}
		return r.executeTasks(ctx, g, rs, engine, packageManager, tracker, startAt)
	}
	return nil
}

func renderDryRunSinglePackageJSON(tasksRun []hashedTask) (string, error) {
	singlePackageTasks := make([]hashedSinglePackageTask, len(tasksRun))
	for i, ht := range tasksRun {
		singlePackageTasks[i] = ht.toSinglePackageTask()
	}
	dryRun := &struct {
		Tasks []hashedSinglePackageTask `json:"tasks"`
	}{singlePackageTasks}
	bytes, err := json.MarshalIndent(dryRun, "", "  ")
	if err != nil {
		return "", errors.Wrap(err, "failed to render JSON")
	}
	return string(bytes), nil
}

func renderDryRunFullJSON(tasksRun []hashedTask, packagesInScope []string) (string, error) {
	dryRun := &struct {
		Packages []string     `json:"packages"`
		Tasks    []hashedTask `json:"tasks"`
	}{
		Packages: packagesInScope,
		Tasks:    tasksRun,
	}
	bytes, err := json.MarshalIndent(dryRun, "", "  ")
	if err != nil {
		return "", errors.Wrap(err, "failed to render JSON")
	}
	return string(bytes), nil
}

func displayDryTextRun(ui cli.Ui, tasksRun []hashedTask, packagesInScope []string, packageInfos map[interface{}]*fs.PackageJSON, isSinglePackage bool) error {
	if !isSinglePackage {
		ui.Output("")
		ui.Info(util.Sprintf("${CYAN}${BOLD}Packages in Scope${RESET}"))
		p := tabwriter.NewWriter(os.Stdout, 0, 0, 1, ' ', 0)
		fmt.Fprintln(p, "Name\tPath\t")
		for _, pkg := range packagesInScope {
			fmt.Fprintf(p, "%s\t%s\t\n", pkg, packageInfos[pkg].Dir)
		}
		if err := p.Flush(); err != nil {
			return err
		}
	}

	ui.Output("")
	ui.Info(util.Sprintf("${CYAN}${BOLD}Tasks to Run${RESET}"))

	for _, task := range tasksRun {
		taskName := task.TaskID
		if isSinglePackage {
			taskName = util.RootTaskTaskName(taskName)
		}
		ui.Info(util.Sprintf("${BOLD}%s${RESET}", taskName))
		w := tabwriter.NewWriter(os.Stdout, 0, 0, 1, ' ', 0)
		fmt.Fprintln(w, util.Sprintf("  ${GREY}Task\t=\t%s\t${RESET}", task.Task))
		var dependencies []string
		var dependents []string
		if !isSinglePackage {
			fmt.Fprintln(w, util.Sprintf("  ${GREY}Package\t=\t%s\t${RESET}", task.Package))
			dependencies = task.Dependencies
			dependents = task.Dependents
		} else {
			dependencies = make([]string, len(task.Dependencies))
			for i, dependency := range task.Dependencies {
				dependencies[i] = util.StripPackageName(dependency)
			}
			dependents = make([]string, len(task.Dependents))
			for i, dependent := range task.Dependents {
				dependents[i] = util.StripPackageName(dependent)
			}
		}
		fmt.Fprintln(w, util.Sprintf("  ${GREY}Hash\t=\t%s\t${RESET}", task.Hash))
		fmt.Fprintln(w, util.Sprintf("  ${GREY}Cached (Local)\t=\t%s\t${RESET}", strconv.FormatBool(task.CacheState.Local)))
		fmt.Fprintln(w, util.Sprintf("  ${GREY}Cached (Remote)\t=\t%s\t${RESET}", strconv.FormatBool(task.CacheState.Remote)))
		if !isSinglePackage {
			fmt.Fprintln(w, util.Sprintf("  ${GREY}Directory\t=\t%s\t${RESET}", task.Dir))
		}
		fmt.Fprintln(w, util.Sprintf("  ${GREY}Command\t=\t%s\t${RESET}", task.Command))
		fmt.Fprintln(w, util.Sprintf("  ${GREY}Outputs\t=\t%s\t${RESET}", strings.Join(task.Outputs, ", ")))
		fmt.Fprintln(w, util.Sprintf("  ${GREY}Log File\t=\t%s\t${RESET}", task.LogFile))
		fmt.Fprintln(w, util.Sprintf("  ${GREY}Dependencies\t=\t%s\t${RESET}", strings.Join(dependencies, ", ")))
		fmt.Fprintln(w, util.Sprintf("  ${GREY}Dependendents\t=\t%s\t${RESET}", strings.Join(dependents, ", ")))
		if err := w.Flush(); err != nil {
			return err
		}
	}
	return nil
}

// filterSinglePackageGraphForDisplay builds an equivalent graph with package names stripped from tasks.
// Given that this should only be used in a single-package context, all of the package names are expected
// to be //. Also, all nodes are always connected to the root node, so we are not concerned with leaving
// behind any unconnected nodes.
func filterSinglePackageGraphForDisplay(originalGraph *dag.AcyclicGraph) *dag.AcyclicGraph {
	graph := &dag.AcyclicGraph{}
	for _, edge := range originalGraph.Edges() {
		src := util.StripPackageName(edge.Source().(string))
		tgt := util.StripPackageName(edge.Target().(string))
		graph.Add(src)
		graph.Add(tgt)
		graph.Connect(dag.BasicEdge(src, tgt))
	}
	return graph
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
	// Daemon-related flags hidden for now, we can unhide when daemon is ready.
	if err := flags.MarkHidden("experimental-use-daemon"); err != nil {
		panic(err)
	}
	if err := flags.MarkHidden("no-daemon"); err != nil {
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

func getDefaultOptions() *Opts {
	return &Opts{
		runOpts: runOpts{
			concurrency: 10,
		},
	}
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

func (r *run) executeTasks(ctx gocontext.Context, g *completeGraph, rs *runSpec, engine *core.Scheduler, packageManager *packagemanager.PackageManager, hashes *taskhash.Tracker, startAt time.Time) error {
	analyticsClient := r.initAnalyticsClient(ctx)
	defer analyticsClient.CloseWithTimeout(50 * time.Millisecond)

	useHTTPCache := !rs.Opts.cacheOpts.SkipRemote
	if useHTTPCache {
		r.base.LogInfo("• Remote caching enabled")
	} else {
		r.base.LogInfo("• Remote caching disabled")
	}

	turboCache, err := r.initCache(ctx, rs, analyticsClient)
	if err != nil {
		if errors.Is(err, cache.ErrNoCachesEnabled) {
			r.base.LogWarning("No caches are enabled. You can try \"turbo login\", \"turbo link\", or ensuring you are not passing --remote-only to enable caching", nil)
		} else {
			return errors.Wrap(err, "failed to set up caching")
		}
	}
	defer func() {
		_ = spinner.WaitFor(ctx, turboCache.Shutdown, r.base.UI, "...writing to cache...", 1500*time.Millisecond)
	}()
	colorCache := colorcache.New()
	runState := NewRunState(startAt, rs.Opts.runOpts.profile)
	runCache := runcache.New(turboCache, r.base.RepoRoot, rs.Opts.runcacheOpts, colorCache)
	ec := &execContext{
		colorCache:      colorCache,
		runState:        runState,
		rs:              rs,
		ui:              &cli.ConcurrentUi{Ui: r.base.UI},
		runCache:        runCache,
		logger:          r.base.Logger,
		packageManager:  packageManager,
		processes:       r.processes,
		taskHashes:      hashes,
		repoRoot:        r.base.RepoRoot,
		isSinglePackage: r.opts.runOpts.singlePackage,
	}

	// run the thing
	execOpts := core.ExecOpts{
		Parallel:    rs.Opts.runOpts.parallel,
		Concurrency: rs.Opts.runOpts.concurrency,
	}
	visitor := g.getPackageTaskVisitor(ctx, func(ctx gocontext.Context, packageTask *nodes.PackageTask) error {
		deps := engine.TaskGraph.DownEdges(packageTask.TaskID)
		return ec.exec(ctx, packageTask, deps)
	})
	errs := engine.Execute(visitor, execOpts)

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
		r.base.UI.Error(err.Error())
	}

	if err := runState.Close(r.base.UI, rs.Opts.runOpts.profile); err != nil {
		return errors.Wrap(err, "error with profiler")
	}
	if exitCode != 0 {
		return &process.ChildExit{
			ExitCode: exitCode,
		}
	}
	return nil
}

type hashedTask struct {
	TaskID          string           `json:"taskId"`
	Task            string           `json:"task"`
	Package         string           `json:"package"`
	Hash            string           `json:"hash"`
	CacheState      cache.ItemStatus `json:"cacheState"`
	Command         string           `json:"command"`
	Outputs         []string         `json:"outputs"`
	ExcludedOutputs []string         `json:"excludedOutputs"`
	LogFile         string           `json:"logFile"`
	Dir             string           `json:"directory"`
	Dependencies    []string         `json:"dependencies"`
	Dependents      []string         `json:"dependents"`
}

func (ht *hashedTask) toSinglePackageTask() hashedSinglePackageTask {
	dependencies := make([]string, len(ht.Dependencies))
	for i, depencency := range ht.Dependencies {
		dependencies[i] = util.StripPackageName(depencency)
	}
	dependents := make([]string, len(ht.Dependents))
	for i, dependent := range ht.Dependents {
		dependents[i] = util.StripPackageName(dependent)
	}
	return hashedSinglePackageTask{
		Task:         util.RootTaskTaskName(ht.TaskID),
		Hash:         ht.Hash,
		Command:      ht.Command,
		Outputs:      ht.Outputs,
		LogFile:      ht.LogFile,
		Dependencies: dependencies,
		Dependents:   dependents,
	}
}

type hashedSinglePackageTask struct {
	Task            string   `json:"task"`
	Hash            string   `json:"hash"`
	Command         string   `json:"command"`
	Outputs         []string `json:"outputs"`
	ExcludedOutputs []string `json:"excludedOutputs"`
	LogFile         string   `json:"logFile"`
	Dependencies    []string `json:"dependencies"`
	Dependents      []string `json:"dependents"`
}

func (r *run) executeDryRun(ctx gocontext.Context, engine *core.Scheduler, g *completeGraph, taskHashes *taskhash.Tracker, rs *runSpec) ([]hashedTask, error) {
	analyticsClient := r.initAnalyticsClient(ctx)
	defer analyticsClient.CloseWithTimeout(50 * time.Millisecond)
	turboCache, err := r.initCache(ctx, rs, analyticsClient)
	defer turboCache.Shutdown()

	if err != nil {
		if errors.Is(err, cache.ErrNoCachesEnabled) {
			r.base.LogWarning("No caches are enabled. You can try \"turbo login\", \"turbo link\", or ensuring you are not passing --remote-only to enable caching", nil)
		} else {
			return nil, errors.Wrap(err, "failed to set up caching")
		}
	}

	taskIDs := []hashedTask{}

	errs := engine.Execute(g.getPackageTaskVisitor(ctx, func(ctx gocontext.Context, packageTask *nodes.PackageTask) error {
		passThroughArgs := rs.ArgsForTask(packageTask.Task)
		deps := engine.TaskGraph.DownEdges(packageTask.TaskID)
		hash, err := taskHashes.CalculateTaskHash(packageTask, deps, passThroughArgs)
		if err != nil {
			return err
		}
		command, ok := packageTask.Command()
		if !ok {
			command = "<NONEXISTENT>"
		}
		isRootTask := packageTask.PackageName == util.RootPkgName
		if isRootTask && commandLooksLikeTurbo(command) {
			return fmt.Errorf("root task %v (%v) looks like it invokes turbo and might cause a loop", packageTask.Task, command)
		}
		ancestors, err := engine.TaskGraph.Ancestors(packageTask.TaskID)
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
		descendents, err := engine.TaskGraph.Descendents(packageTask.TaskID)
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

		itemStatus, err := turboCache.Exists(hash)
		if err != nil {
			return err
		}

		taskIDs = append(taskIDs, hashedTask{
			TaskID:          packageTask.TaskID,
			Task:            packageTask.Task,
			Package:         packageTask.PackageName,
			Hash:            hash,
			CacheState:      itemStatus,
			Command:         command,
			Dir:             packageTask.Pkg.Dir.ToString(),
			Outputs:         packageTask.TaskDefinition.Outputs.Inclusions,
			ExcludedOutputs: packageTask.TaskDefinition.Outputs.Exclusions,
			LogFile:         packageTask.RepoRelativeLogFile(),
			Dependencies:    stringAncestors,
			Dependents:      stringDescendents,
		})

		return nil
	}), core.ExecOpts{
		Concurrency: 1,
		Parallel:    false,
	})
	if len(errs) > 0 {
		for _, err := range errs {
			r.base.UI.Error(err.Error())
		}
		return nil, errors.New("errors occurred during dry-run graph traversal")
	}
	return taskIDs, nil
}

var _isTurbo = regexp.MustCompile(fmt.Sprintf("(?:^|%v|\\s)turbo(?:$|\\s)", regexp.QuoteMeta(string(filepath.Separator))))

func commandLooksLikeTurbo(command string) bool {
	return _isTurbo.MatchString(command)
}

func validateTasks(pipeline fs.Pipeline, tasks []string) error {
	for _, task := range tasks {
		if !pipeline.HasTask(task) {
			return fmt.Errorf("task `%v` not found in turbo `pipeline` in \"turbo.json\". Are you sure you added it?", task)
		}
	}
	return nil
}

type execContext struct {
	colorCache      *colorcache.ColorCache
	runState        *RunState
	rs              *runSpec
	ui              cli.Ui
	runCache        *runcache.RunCache
	logger          hclog.Logger
	packageManager  *packagemanager.PackageManager
	processes       *process.Manager
	taskHashes      *taskhash.Tracker
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

func (ec *execContext) exec(ctx gocontext.Context, packageTask *nodes.PackageTask, deps dag.Set) error {
	cmdTime := time.Now()

	prefix := packageTask.OutputPrefix(ec.isSinglePackage)
	prettyPrefix := ec.colorCache.PrefixWithColor(packageTask.PackageName, prefix)

	targetLogger := ec.logger.Named(prefix)
	targetLogger.Debug("start")

	// Setup tracer
	tracer := ec.runState.Run(packageTask.TaskID)

	// Create a logger
	targetUI := &cli.PrefixedUi{
		Ui:           ec.ui,
		OutputPrefix: prettyPrefix,
		InfoPrefix:   prettyPrefix,
		ErrorPrefix:  prettyPrefix,
		WarnPrefix:   prettyPrefix,
	}

	passThroughArgs := ec.rs.ArgsForTask(packageTask.Task)
	hash, err := ec.taskHashes.CalculateTaskHash(packageTask, deps, passThroughArgs)
	ec.logger.Debug("task hash", "value", hash)
	if err != nil {
		ec.ui.Error(fmt.Sprintf("Hashing error: %v", err))
		// @TODO probably should abort fatally???
	}
	// TODO(gsoltis): if/when we fix https://github.com/vercel/turborepo/issues/937
	// the following block should never get hit. In the meantime, keep it after hashing
	// so that downstream tasks can count on the hash existing
	//
	// bail if the script doesn't exist
	if _, ok := packageTask.Command(); !ok {
		targetLogger.Debug("no task in package, skipping")
		targetLogger.Debug("done", "status", "skipped", "duration", time.Since(cmdTime))
		return nil
	}
	// Cache ---------------------------------------------
	taskCache := ec.runCache.TaskCache(packageTask, hash)
	hit, err := taskCache.RestoreOutputs(ctx, targetUI, targetLogger)
	if err != nil {
		targetUI.Error(fmt.Sprintf("error fetching from cache: %s", err))
	} else if hit {
		tracer(TargetCached, nil)
		return nil
	}
	// Setup command execution
	argsactual := append([]string{"run"}, packageTask.Task)
	if len(passThroughArgs) > 0 {
		// This will be either '--' or a typed nil
		argsactual = append(argsactual, ec.packageManager.ArgSeparator...)
		argsactual = append(argsactual, passThroughArgs...)
	}

	cmd := exec.Command(ec.packageManager.Command, argsactual...)
	// TODO: repoRoot probably should be AbsoluteSystemPath, but it's Join method
	// takes a RelativeSystemPath. Resolve during migration from turbopath.AbsoluteSystemPath to
	// AbsoluteSystemPath
	cmd.Dir = ec.repoRoot.UntypedJoin(packageTask.Pkg.Dir.ToStringDuringMigration()).ToString()
	envs := fmt.Sprintf("TURBO_HASH=%v", hash)
	cmd.Env = append(os.Environ(), envs)

	// Setup stdout/stderr
	// If we are not caching anything, then we don't need to write logs to disk
	// be careful about this conditional given the default of cache = true
	writer, err := taskCache.OutputWriter(prefix)
	if err != nil {
		tracer(TargetBuildFailed, err)
		ec.logError(targetLogger, prettyPrefix, err)
		if !ec.rs.Opts.runOpts.continueOnError {
			os.Exit(1)
		}
	}
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
			return nil
		}
		tracer(TargetBuildFailed, err)
		targetLogger.Error(fmt.Sprintf("Error: command finished with error: %v", err))
		if !ec.rs.Opts.runOpts.continueOnError {
			targetUI.Error(fmt.Sprintf("ERROR: command finished with error: %s", err))
			ec.processes.Close()
		} else {
			targetUI.Warn("command finished with error, but continuing...")
		}
		return err
	}

	duration := time.Since(cmdTime)
	// Close off our outputs and cache them
	if err := closeOutputs(); err != nil {
		ec.logError(targetLogger, "", err)
	} else {
		if err = taskCache.SaveOutputs(ctx, targetLogger, targetUI, int(duration.Milliseconds())); err != nil {
			ec.logError(targetLogger, "", fmt.Errorf("error caching output: %w", err))
		}
	}

	// Clean up tracing
	tracer(TargetBuilt, nil)
	targetLogger.Debug("done", "status", "complete", "duration", duration)
	return nil
}

func (g *completeGraph) getPackageTaskVisitor(ctx gocontext.Context, visitor func(ctx gocontext.Context, packageTask *nodes.PackageTask) error) func(taskID string) error {
	return func(taskID string) error {

		name, task := util.GetPackageTaskFromId(taskID)
		pkg, ok := g.PackageInfos[name]
		if !ok {
			return fmt.Errorf("cannot find package %v for task %v", name, taskID)
		}

		// first check for package-tasks
		taskDefinition, ok := g.Pipeline[fmt.Sprintf("%v", taskID)]
		if !ok {
			// then check for regular tasks
			fallbackTaskDefinition, notcool := g.Pipeline[task]
			// if neither, then bail
			if !notcool && !ok {
				return nil
			}
			// override if we need to...
			taskDefinition = fallbackTaskDefinition
		}
		return visitor(ctx, &nodes.PackageTask{
			TaskID:         taskID,
			Task:           task,
			PackageName:    name,
			Pkg:            pkg,
			TaskDefinition: &taskDefinition,
		})
	}
}
