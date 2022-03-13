package run

import (
	"bufio"
	gocontext "context"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"text/tabwriter"
	"time"

	"github.com/fatih/color"
	"github.com/hashicorp/go-hclog"
	"github.com/pkg/errors"
	"github.com/pyr-sh/dag"
	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/analytics"
	"github.com/vercel/turborepo/cli/internal/api"
	"github.com/vercel/turborepo/cli/internal/cache"
	"github.com/vercel/turborepo/cli/internal/cmdutil"
	"github.com/vercel/turborepo/cli/internal/context"
	"github.com/vercel/turborepo/cli/internal/core"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/globby"
	"github.com/vercel/turborepo/cli/internal/logger"
	"github.com/vercel/turborepo/cli/internal/logstreamer"
	"github.com/vercel/turborepo/cli/internal/process"
	"github.com/vercel/turborepo/cli/internal/run"
	"github.com/vercel/turborepo/cli/internal/scm"
	"github.com/vercel/turborepo/cli/internal/scope"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"
	"github.com/vercel/turborepo/cli/internal/util/browser"
)

const (
	TOPOLOGICAL_PIPELINE_DELIMITER = "^"
	ENV_PIPELINE_DELIMITER         = "$"
)

// completeGraph represents the common state inferred from the filesystem and pipeline.
// It is not intended to include information specific to a particular run.
type completeGraph struct {
	TopologicalGraph dag.AcyclicGraph
	Pipeline         map[string]fs.Pipeline
	SCC              [][]dag.Vertex
	PackageInfos     map[interface{}]*fs.PackageJSON
	GlobalHash       string
	RootNode         string
}

// runSpec contains the run-specific configuration elements that come from a particular
// invocation of turbo.
type runSpec struct {
	Targets      []string
	FilteredPkgs util.Set
	Opts         *run.RunOptions
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

type execContext struct {
	colorCache         *run.ColorCache
	runState           *run.RunState
	rs                 *runSpec
	logReplayWaitGroup sync.WaitGroup
	logger             *logger.ConcurrentLogger
	turboCache         cache.Cache
	hlogger            hclog.Logger
	backend            *api.LanguageBackend
	processes          *process.Manager
}

type packageTask struct {
	taskID      string
	task        string
	packageName string
	pkg         *fs.PackageJSON
	pipeline    *fs.Pipeline
}

func (rs *runSpec) ArgsForTask(task string) []string {
	passThroughArgs := make([]string, 0, len(rs.Opts.PassThroughArgs))
	for _, target := range rs.Targets {
		if target == task {
			passThroughArgs = append(passThroughArgs, rs.Opts.PassThroughArgs...)
		}
	}
	return passThroughArgs
}

func (e *execContext) logError(prefix, format string, args ...interface{}) {
	err := fmt.Errorf(format, args...)
	e.hlogger.Error(prefix, "error", err)
	e.logger.Printf("%w", fmt.Errorf("%s%s", prefix, color.RedString(" %v", err)))
}

func RunCmd(ch *cmdutil.Helper) *cobra.Command {
	passThroughArgs := strings.SplitN(strings.Join(os.Args, " "), " -- ", 2)
	opts := &run.RunOptions{
		Bail:              true,
		IncludeDependents: true,
	}

	cmd := &cobra.Command{
		Use:   "run",
		Short: "Run tasks across projects in your monorepo",
		Long: `Run tasks across projects in your monorepo.

By default, turbo executes tasks in topological order (i.e.
dependencies first) and then caches the results. Re-running commands for
tasks already in the cache will skip re-execution and immediately move
artifacts from the cache into the correct output folders (as if the task
occurred again).
`,
		Args: cobra.MinimumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			// Force streaming output in CI/CD non-interactive mode
			if !logger.IsTTY || logger.IsCI {
				opts.Stream = true
			}
			// We can only set this cache folder after we know actual cwd
			opts.CacheDir = filepath.Join(opts.Cwd, opts.CacheDir)
			if !opts.Graph {
				opts.DotGraph = ""
			} else {
				if opts.DotGraph == "" {
					opts.DotGraph = fmt.Sprintf("graph-%v.jpg", time.Now().UnixNano())
				}
			}
			if opts.DryRunType != "" {
				opts.DryRun = true
			}
			if len(opts.Scope) != 0 && opts.Since != "" && !opts.IncludeDeps {
				opts.IncludeDeps = true
			}
			if len(passThroughArgs) == 2 {
				opts.PassThroughArgs = strings.Split(passThroughArgs[1], " ")
			}

			startAt := time.Now()
			log.SetFlags(0)

			ch.Config.Cache.Dir = opts.CacheDir

			ctx, err := context.New(context.WithGraph(opts.Cwd, ch.Config))
			if err != nil {
				return ch.LogError("%w", err)
			}
			targets, err := getTargetsFromArguments(args, ctx.TurboConfig)
			if err != nil {
				return ch.LogError("failed to resolve targets: %w", err)
			}

			scmInstance, err := scm.FromInRepo(opts.Cwd)
			if err != nil {
				if errors.Is(err, scm.ErrFallback) {
					ch.LogWarning("%w", err)
				} else {
					return ch.LogError("failed to create SCM: %w", err)
				}
			}

			filteredPkgs, err := scope.ResolvePackages(opts.ScopeOpts(), scmInstance, ctx, ch.Logger, ch.Config.Logger)
			if err != nil {
				ch.LogError("failed resolve packages to run %v", err)
			}

			ch.Config.Logger.Debug("global hash", "value", ctx.GlobalHash)
			ch.Config.Logger.Debug("local cache folder", "path", opts.CacheDir)
			fs.EnsureDir(opts.CacheDir)

			// TODO: consolidate some of these arguments
			g := &completeGraph{
				TopologicalGraph: ctx.TopologicalGraph,
				Pipeline:         ctx.TurboConfig.Pipeline,
				SCC:              ctx.SCC,
				PackageInfos:     ctx.PackageInfos,
				GlobalHash:       ctx.GlobalHash,
				RootNode:         ctx.RootNode,
			}
			rs := &runSpec{
				Targets:      targets,
				FilteredPkgs: filteredPkgs,
				Opts:         opts,
			}
			backend := ctx.Backend
			return runOperation(ch, g, rs, backend, startAt)
		},
	}

	path, err := os.Getwd()
	if err != nil {
		return nil
	}

	cmd.Flags().StringArrayVar(&opts.Scope, "scope", []string{}, "package(s) to act as entry points for task execution, supports globs")
	cmd.Flags().StringVar(&opts.CacheDir, "cache-dir", filepath.FromSlash("./node_modules/.cache/turbo"), "Specify local filesystem cache directory")
	cmd.Flags().IntVar(&opts.Concurrency, "concurrency", 10, "concurrency of task execution")
	cmd.Flags().BoolVar(&opts.ShouldContinue, "continue", false, "continue execution even if a task exits with an error or non-zero exit code")
	cmd.Flags().BoolVarP(&opts.Force, "force", "f", false, "ignore the existing cache")
	cmd.Flags().StringVar(&opts.Profile, "profile", "", "file to write turbo's performance profile output into")
	cmd.Flags().BoolVarP(&opts.Graph, "graph", "g", false, "generate a Dot graph of the task execution")
	cmd.Flags().StringVar(&opts.DotGraph, "graph-path", "", "path for Dot graph")
	cmd.Flags().StringArrayVar(&opts.GlobalDeps, "global-deps", []string{}, "glob of global filesystem dependencies to be hashed")
	cmd.Flags().StringVar(&opts.Since, "since", "", "limit/set scope to changed packages since a mergebase")
	cmd.Flags().StringArrayVar(&opts.Ignore, "ignore", []string{}, "files to ignore when calculating changed files, supports globs")
	cmd.Flags().BoolVarP(&opts.Parallel, "parallel", "p", false, "execute all tasks in parallel")
	cmd.Flags().BoolVar(&opts.IncludeDeps, "include-deps", false, "include the dependencies of tasks in execution")
	cmd.Flags().BoolVar(&opts.NoDeps, "no-deps", false, "exclude dependent task consumers from execution")
	cmd.Flags().BoolVar(&opts.NoCache, "no-cache", false, "avoid saving task results to the cache")
	cmd.Flags().StringVar(&opts.DryRunType, "dry-run", "", "don't actually run tasks")
	cmd.Flags().StringVar(&opts.Cwd, "cwd", path, "directory to execute command in")
	cmd.Flags().BoolVar(&opts.Stream, "stream", true, "stream???")
	cmd.Flags().BoolVar(&opts.Only, "only", true, "only???")

	cmd.Flags().MarkHidden("stream")
	cmd.Flags().MarkHidden("only")

	return cmd
}

func runOperation(ch *cmdutil.Helper, g *completeGraph, rs *runSpec, backend *api.LanguageBackend, startAt time.Time) error {
	var topoVisit []interface{}
	for _, node := range g.SCC {
		v := node[0]
		if v == g.RootNode {
			continue
		}
		topoVisit = append(topoVisit, v)
		pack := g.PackageInfos[v]

		ancestralHashes := make([]string, 0, len(pack.InternalDeps))
		if len(pack.InternalDeps) > 0 {
			for _, ancestor := range pack.InternalDeps {
				if h, ok := g.PackageInfos[ancestor]; ok {
					ancestralHashes = append(ancestralHashes, h.Hash)
				}
			}
			sort.Strings(ancestralHashes)
		}
		var hashable = struct {
			hashOfFiles      string
			ancestralHashes  []string
			externalDepsHash string
			globalHash       string
		}{hashOfFiles: pack.FilesHash, ancestralHashes: ancestralHashes, externalDepsHash: pack.ExternalDepsHash, globalHash: g.GlobalHash}

		var err error
		pack.Hash, err = fs.HashObject(hashable)
		if err != nil {
			ch.LogError("%v: error computing combined hash: %v", pack.Name, err)
		}
		ch.Config.Logger.Debug(fmt.Sprintf("%v: package ancestralHash", pack.Name), "hash", ancestralHashes)
		ch.Config.Logger.Debug(fmt.Sprintf("%v: package hash", pack.Name), "hash", pack.Hash)
	}

	ch.Config.Logger.Debug("topological sort order", "value", topoVisit)

	vertexSet := make(util.Set)
	for _, v := range g.TopologicalGraph.Vertices() {
		vertexSet.Add(v)
	}
	// We remove nodes that aren't in the final filter set
	for _, toRemove := range vertexSet.Difference(rs.FilteredPkgs) {
		if toRemove != g.RootNode {
			g.TopologicalGraph.Remove(toRemove)
		}
	}

	// If we are running in parallel, then we remove all the edges in the graph
	// except for the root
	if rs.Opts.Parallel {
		for _, edge := range g.TopologicalGraph.Edges() {
			if edge.Target() != g.RootNode {
				g.TopologicalGraph.RemoveEdge(edge)
			}
		}
	}

	engine, err := buildTaskGraph(&g.TopologicalGraph, g.Pipeline, rs)
	if err != nil {
		return ch.LogError("preparing engine: %s", err)
	}

	if rs.Opts.DotGraph != "" {
		err := generateDotGraph(ch, engine.TaskGraph, filepath.Join(rs.Opts.Cwd, rs.Opts.DotGraph))
		if err != nil {
			return ch.LogError(err.Error())
		}
	} else if rs.Opts.DryRun {
		tasksRun, err := executeDryRun(ch, engine, g, rs, ch.Config.Logger)
		if err != nil {
			return ch.LogError(err.Error())
		}

		packagesInScope := rs.FilteredPkgs.UnsafeListOfStrings()
		sort.Strings(packagesInScope)
		if rs.Opts.DryRunType == "json" {
			dryRun := &struct {
				Packages []string     `json:"packages"`
				Tasks    []hashedTask `json:"tasks"`
			}{
				Packages: packagesInScope,
				Tasks:    tasksRun,
			}

			bytes, err := json.MarshalIndent(dryRun, "", "  ")
			if err != nil {
				return ch.LogError("failed to render to JSON: %w", err)
			}

			ch.Logger.Printf(string(bytes))
		} else {
			ch.Logger.Printf("")
			ch.Logger.Printf("${CYAN}${BOLD}Packages in Scope${RESET}")

			p := tabwriter.NewWriter(os.Stdout, 0, 0, 1, ' ', 0)
			fmt.Fprintln(p, "Name\tPath\t")
			for _, pkg := range packagesInScope {
				fmt.Fprintf(p, "%s\t%s\t", pkg, g.PackageInfos[pkg].Dir)
			}
			p.Flush()

			ch.Logger.Printf("")
			ch.Logger.Printf("${CYAN}${BOLD}Tasks to Run${RESET}")

			for _, task := range tasksRun {
				ch.Logger.Printf("${BOLD}%s${RESET}", task.TaskID)
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

		ch.Logger.Printf(ui.Dim("• Packages in scope: %v"), strings.Join(packagesInScope, ", "))
		if rs.Opts.Stream {
			ch.Logger.Printf("%s %s %s", ui.Dim("• Running"), ui.Dim(ui.Bold(strings.Join(rs.Targets, ", "))), ui.Dim(fmt.Sprintf("in %v packages", rs.FilteredPkgs.Len())))
		}

		return executeTasks(ch, g, rs, engine, backend, startAt)
	}

	return nil
}

func buildTaskGraph(topoGraph *dag.AcyclicGraph, pipeline map[string]fs.Pipeline, rs *runSpec) (*core.Scheduler, error) {
	engine := core.NewScheduler(topoGraph)
	for taskName, value := range pipeline {
		topoDeps := make(util.Set)
		deps := make(util.Set)
		if util.IsPackageTask(taskName) {
			for _, from := range value.DependsOn {
				if strings.HasPrefix(from, ENV_PIPELINE_DELIMITER) {
					continue
				}
				if util.IsPackageTask(from) {
					engine.AddDep(from, taskName)
					continue
				} else if strings.Contains(from, TOPOLOGICAL_PIPELINE_DELIMITER) {
					topoDeps.Add(from[1:])
				} else {
					deps.Add(from)
				}
			}
			_, id := util.GetPackageTaskFromId(taskName)
			taskName = id
		} else {
			for _, from := range value.DependsOn {
				if strings.HasPrefix(from, ENV_PIPELINE_DELIMITER) {
					continue
				}
				if strings.Contains(from, TOPOLOGICAL_PIPELINE_DELIMITER) {
					topoDeps.Add(from[1:])
				} else {
					deps.Add(from)
				}
			}
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
		TasksOnly: rs.Opts.Only,
	}); err != nil {
		return nil, err
	}
	return engine, nil
}

func hasGraphViz() bool {
	err := exec.Command("dot", "-v").Run()
	return err == nil
}

func executeTasks(ch *cmdutil.Helper, g *completeGraph, rs *runSpec, engine *core.Scheduler, backend *api.LanguageBackend, startAt time.Time) error {
	goctx := gocontext.Background()

	var analyticsSink analytics.Sink
	if ch.Config.IsAuthenticated() {
		analyticsSink = ch.Config.ApiClient
	} else {
		analyticsSink = analytics.NullSink
	}

	analyticsClient := analytics.NewClient(goctx, analyticsSink, ch.Config.Logger.Named("analytics"))
	defer analyticsClient.CloseWithTimeout(50 * time.Millisecond)

	turboCache := cache.New(ch.Config, analyticsClient)
	defer turboCache.Shutdown()

	clogger := logger.NewConcurrent(ch.Logger)
	runState := run.NewRunState(rs.Opts, startAt)
	runState.Listen(ch.Logger, time.Now())
	ec := &execContext{
		colorCache: run.NewColorCache(),
		runState:   runState,
		rs:         rs,
		logger:     clogger,
		turboCache: turboCache,
		hlogger:    ch.Config.Logger,
		backend:    backend,
		processes:  ch.Processes,
	}

	// run the thing
	errs := engine.Execute(g.getPackageTaskVisitor(ec.exec), core.ExecOpts{
		Parallel:    rs.Opts.Parallel,
		Concurrency: rs.Opts.Concurrency,
	})

	// Track if we saw any child with a non-zero exit code
	exitCode := 0
	exitCodeErr := &process.ChildExit{}
	for _, err := range errs {
		if errors.As(err, &exitCodeErr) {
			if exitCodeErr.ExitCode > exitCode {
				exitCode = exitCodeErr.ExitCode
			}
		}
		ch.Logger.Printf("%v", ch.Logger.Errorf(err.Error()))
	}

	ec.logReplayWaitGroup.Wait()

	if err := runState.Close(ch.Logger, rs.Opts.Profile); err != nil {
		return &cmdutil.Error{
			ExitCode: exitCode,
			Err:      ch.Logger.Errorf("error with profiler: %s", err.Error()),
		}
	}

	return nil
}

func executeDryRun(ch *cmdutil.Helper, engine *core.Scheduler, g *completeGraph, rs *runSpec, logger hclog.Logger) ([]hashedTask, error) {
	taskIDs := []hashedTask{}
	errs := engine.Execute(g.getPackageTaskVisitor(func(pt *packageTask) error {
		command, ok := pt.pkg.Scripts[pt.task]
		if !ok {
			logger.Debug("no task in package, skipping")
			logger.Debug("done", "status", "skipped")
			return nil
		}
		passThroughArgs := rs.ArgsForTask(pt.task)
		hash, err := pt.hash(passThroughArgs, logger)
		if err != nil {
			return err
		}
		ancestors, err := engine.TaskGraph.Ancestors(pt.taskID)
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
		descendents, err := engine.TaskGraph.Descendents(pt.taskID)
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
			TaskID:       pt.taskID,
			Task:         pt.task,
			Package:      pt.packageName,
			Hash:         hash,
			Command:      command,
			Dir:          pt.pkg.Dir,
			Outputs:      pt.ExternalOutputs(),
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
			ch.Logger.Errorf(err.Error())
		}
		return nil, errors.New("errors occurred during dry-run graph traversal")
	}
	return taskIDs, nil
}

// Replay logs will try to replay logs back to the stdout
func replayLogs(logger hclog.Logger, cLogger *logger.ConcurrentLogger, runOptions *run.RunOptions, logFileName, hash string, wg *sync.WaitGroup, silent bool) {
	defer wg.Done()
	logger.Debug("start replaying logs")
	f, err := os.Open(filepath.Join(runOptions.Cwd, logFileName))
	if err != nil && !silent {
		cLogger.Printf("%v", cLogger.Warnf("error reading logs: %v", err))
		logger.Error(fmt.Sprintf("error reading logs: %v", err.Error()))
	}
	defer f.Close()
	scan := bufio.NewScanner(f)
	for scan.Scan() {
		cLogger.Printf(ui.StripAnsi(string(scan.Bytes()))) // Writing to Stdout
	}
	logger.Debug("finish replaying logs")
}

// GetTargetsFromArguments returns a list of targets from the arguments and Turbo config.
// Return targets are always unique sorted alphabetically.
func getTargetsFromArguments(arguments []string, configJson *fs.TurboConfigJSON) ([]string, error) {
	targets := make(util.Set)
	for _, arg := range arguments {
		if arg == "--" {
			break
		}
		if !strings.HasPrefix(arg, "-") {
			targets.Add(arg)
			found := false
			for task := range configJson.Pipeline {
				if task == arg {
					found = true
				}
			}
			if !found {
				return nil, fmt.Errorf("task `%v` not found in turbo pipeline in package.json. Are you sure you added it?", arg)
			}
		}
	}
	stringTargets := targets.UnsafeListOfStrings()
	sort.Strings(stringTargets)
	return stringTargets, nil
}

func (e *execContext) exec(pt *packageTask) error {
	cmdTime := time.Now()

	targetHlogger := e.hlogger.Named(fmt.Sprintf("%v:%v", pt.pkg.Name, pt.task))
	targetHlogger.Debug("start")

	// bail if the script doesn't exist
	if _, ok := pt.pkg.Scripts[pt.task]; !ok {
		targetHlogger.Debug("no task in package, skipping")
		targetHlogger.Debug("done", "status", "skipped", "duration", time.Since(cmdTime))
		return nil
	}

	// Setup tracer
	tracer := e.runState.Run(util.GetTaskId(pt.pkg.Name, pt.task))

	// Create a logger
	pref := e.colorCache.PrefixColor(pt.pkg.Name)
	actualPrefix := pref("%s:%s: ", pt.pkg.Name, pt.task)
	targetLogger := logger.NewPrefixed(actualPrefix, actualPrefix, actualPrefix, actualPrefix)

	logFileName := filepath.Join(pt.pkg.Dir, ".turbo", fmt.Sprintf("turbo-%v.log", pt.task))
	targetHlogger.Debug("log file", "path", filepath.Join(e.rs.Opts.Cwd, logFileName))

	passThroughArgs := e.rs.ArgsForTask(pt.task)
	hash, err := pt.hash(passThroughArgs, e.hlogger)
	e.hlogger.Debug("task hash", "value", hash)
	if err != nil {
		targetLogger.Printf("%w", targetLogger.Errorf("hashing error: %v", err))
		// @TODO probably should abort fatally???
	}
	// Cache ---------------------------------------------
	var hit bool
	if !e.rs.Opts.Force {
		hit, _, _, err = e.turboCache.Fetch(e.rs.Opts.Cwd, hash, nil)
		if err != nil {
			targetLogger.Printf("%w", targetLogger.Errorf(fmt.Sprintf("error fetching from cache: %s", err)))
		} else if hit {
			if e.rs.Opts.Stream && fs.FileExists(filepath.Join(e.rs.Opts.Cwd, logFileName)) {
				e.logReplayWaitGroup.Add(1)
				go replayLogs(targetHlogger, e.logger, e.rs.Opts, logFileName, hash, &e.logReplayWaitGroup, false)
			}
			targetHlogger.Debug("done", "status", "complete", "duration", time.Since(cmdTime))
			tracer(run.TargetCached, nil)

			return nil
		}
		if e.rs.Opts.Stream {
			targetLogger.Output("cache miss, executing %s", ui.Dim(hash))
		}
	} else {
		if e.rs.Opts.Stream {
			targetLogger.Output("cache bypass, force executing %s", ui.Dim(hash))
		}
	}

	// Setup command execution
	argsactual := append([]string{"run"}, pt.task)
	argsactual = append(argsactual, passThroughArgs...)
	// @TODO: @jaredpalmer fix this hack to get the package manager's name
	var cmd *exec.Cmd
	if e.backend.Name == "nodejs-berry" {
		cmd = exec.Command("yarn", argsactual...)
	} else {
		cmd = exec.Command(strings.TrimPrefix(e.backend.Name, "nodejs-"), argsactual...)
	}
	cmd.Dir = pt.pkg.Dir
	envs := fmt.Sprintf("TURBO_HASH=%v", hash)
	cmd.Env = append(os.Environ(), envs)

	// Setup stdout/stderr
	// If we are not caching anything, then we don't need to write logs to disk
	// be careful about this conditional given the default of cache = true
	var writer io.Writer
	if e.rs.Opts.NoCache || (pt.pipeline.Cache != nil && !*pt.pipeline.Cache) {
		writer = os.Stdout
	} else {
		// Setup log file
		if err := fs.EnsureDir(logFileName); err != nil {
			tracer(run.TargetBuildFailed, err)
			e.logError(actualPrefix, "%w", err)
			if e.rs.Opts.Bail {
				os.Exit(1)
			}
		}
		output, err := os.Create(logFileName)
		if err != nil {
			tracer(run.TargetBuildFailed, err)
			e.logError(actualPrefix, "%w", err)
			if e.rs.Opts.Bail {
				os.Exit(1)
			}
		}
		defer output.Close()
		bufWriter := bufio.NewWriter(output)
		bufWriter.WriteString(fmt.Sprintf("%scache hit, replaying output %s\n", actualPrefix, ui.Dim(hash)))
		defer bufWriter.Flush()
		writer = io.MultiWriter(os.Stdout, bufWriter)
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

	// Run the command
	if err := e.processes.Exec(cmd); err != nil {
		// if we already know we're in the process of exiting,
		// we don't need to record an error to that effect.
		if errors.Is(err, process.ErrClosing) {
			return nil
		}
		tracer(run.TargetBuildFailed, err)
		targetHlogger.Error("Error: command finished with error: %w", err)
		if e.rs.Opts.Bail {
			if e.rs.Opts.Stream {
				targetLogger.Printf("%s", fmt.Errorf("%sError: command finished with error: %w", actualPrefix, err).Error())
			} else {
				f, err := os.Open(filepath.Join(e.rs.Opts.Cwd, logFileName))
				if err != nil {
					targetLogger.Printf("%w", targetLogger.Warnf("failed reading logs: %v", err))
				}
				defer f.Close()
				scan := bufio.NewScanner(f)
				targetLogger.Printf("%w", targetLogger.Errorf(""))
				targetLogger.Printf("%w", targetLogger.Errorf("%s ${RED}%s finished with error${RESET}", ui.ERROR_PREFIX, util.GetTaskId(pt.pkg.Name, pt.task)))
				targetLogger.Printf("%w", targetLogger.Errorf(""))
				for scan.Scan() {
					e.logger.Printf("${RED}%s:%s: ${RESET}%s", pt.pkg.Name, pt.task, scan.Bytes()) // Writing to Stdout
				}
			}
			e.processes.Close()
		} else {
			if e.rs.Opts.Stream {
				targetLogger.Printf("%w", targetLogger.Warnf("command finished with error, but continuing..."))
			}
		}
		return err
	}

	// Cache command outputs
	if !e.rs.Opts.NoCache && (pt.pipeline.Cache == nil || *pt.pipeline.Cache) {
		outputs := pt.HashableOutputs()
		targetHlogger.Debug("caching output", "outputs", outputs)
		ignore := []string{}
		filesToBeCached := globby.GlobFiles(pt.pkg.Dir, outputs, ignore)
		if err := e.turboCache.Put(pt.pkg.Dir, hash, int(time.Since(cmdTime).Milliseconds()), filesToBeCached); err != nil {
			e.logError("", "error caching output: %w", err)
		}
	}

	// Clean up tracing
	tracer(run.TargetBuilt, nil)
	targetHlogger.Debug("done", "status", "complete", "duration", time.Since(cmdTime))
	return nil
}

func generateDotGraph(ch *cmdutil.Helper, taskGraph *dag.AcyclicGraph, outputFilename string) error {
	graphString := string(taskGraph.Dot(&dag.DotOpts{
		Verbose:    true,
		DrawCycles: true,
	}))
	ext := filepath.Ext(outputFilename)
	if ext == ".html" {
		f, err := os.Create(outputFilename)
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
		ch.Logger.Printf("")
		ch.Logger.Printf(fmt.Sprintf("✔ Generated task graph in %s", ui.Bold(outputFilename)))
		if ui.IsTTY {
			browser.OpenBrowser(outputFilename)
		}
		return nil
	}
	hasDot := hasGraphViz()
	if hasDot {
		dotArgs := []string{"-T" + ext[1:], "-o", outputFilename}
		cmd := exec.Command("dot", dotArgs...)
		cmd.Stdin = strings.NewReader(graphString)
		if err := cmd.Run(); err != nil {
			return fmt.Errorf("could not generate task graphfile %v:  %w", outputFilename, err)
		} else {
			ch.Logger.Printf("")
			ch.Logger.Printf("✔ Generated task graph in %s", ui.Bold(outputFilename))
		}
	} else {
		ch.Logger.Printf("")
		ch.Logger.Printf("%v", ch.LogWarning("`turbo` uses Graphviz to generate an image of your\ngraph, but Graphviz isn't installed on this machine.\n\nYou can download Graphviz from https://graphviz.org/download.\n\nIn the meantime, you can use this string output with an\nonline Dot graph viewer."))
		ch.Logger.Printf("")
		ch.Logger.Printf(graphString)
	}
	return nil
}

func (pt *packageTask) ExternalOutputs() []string {
	if pt.pipeline.Outputs == nil {
		return []string{"dist/**/*", "build/**/*"}
	}
	return pt.pipeline.Outputs
}

func (pt *packageTask) RepoRelativeLogFile() string {
	return filepath.Join(pt.pkg.Dir, ".turbo", fmt.Sprintf("turbo-%v.log", pt.task))
}

func (pt *packageTask) HashableOutputs() []string {
	outputs := []string{fmt.Sprintf(".turbo/turbo-%v.log", pt.task)}
	outputs = append(outputs, pt.ExternalOutputs()...)
	return outputs
}

func (pt *packageTask) hash(args []string, logger hclog.Logger) (string, error) {
	// Hash ---------------------------------------------
	outputs := pt.HashableOutputs()
	logger.Debug("task output globs", "outputs", outputs)

	// Hash the task-specific environment variables found in the dependsOnKey in the pipeline
	var hashableEnvVars []string
	var hashableEnvPairs []string
	if len(pt.pipeline.DependsOn) > 0 {
		for _, v := range pt.pipeline.DependsOn {
			if strings.Contains(v, ENV_PIPELINE_DELIMITER) {
				trimmed := strings.TrimPrefix(v, ENV_PIPELINE_DELIMITER)
				hashableEnvPairs = append(hashableEnvPairs, fmt.Sprintf("%v=%v", trimmed, os.Getenv(trimmed)))
				hashableEnvVars = append(hashableEnvVars, trimmed)
			}
		}
		sort.Strings(hashableEnvVars) // always sort them
	}
	logger.Debug("hashable env vars", "vars", hashableEnvVars)
	hashable := struct {
		Hash             string
		Task             string
		Outputs          []string
		PassThruArgs     []string
		HashableEnvPairs []string
	}{
		Hash:             pt.pkg.Hash,
		Task:             pt.task,
		Outputs:          outputs,
		PassThruArgs:     args,
		HashableEnvPairs: hashableEnvPairs,
	}
	return fs.HashObject(hashable)
}

func (g *completeGraph) getPackageTaskVisitor(visitor func(pt *packageTask) error) func(taskID string) error {
	return func(taskID string) error {
		name, task := util.GetPackageTaskFromId(taskID)
		pkg := g.PackageInfos[name]
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
		return visitor(&packageTask{
			taskID:      taskID,
			task:        task,
			packageName: name,
			pkg:         pkg,
			pipeline:    &pipeline,
		})
	}
}
