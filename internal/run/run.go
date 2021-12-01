package run

import (
	"bufio"
	"flag"
	"fmt"
	"io"
	"log"
	"os"
	"os/exec"
	"path"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"
	"turbo/internal/cache"
	"turbo/internal/config"
	"turbo/internal/context"
	"turbo/internal/core"
	"turbo/internal/fs"
	"turbo/internal/scm"
	"turbo/internal/ui"
	"turbo/internal/util"

	"github.com/bmatcuk/doublestar"
	"github.com/pyr-sh/dag"

	"github.com/fatih/color"
	glob "github.com/gobwas/glob"
	"github.com/hashicorp/go-hclog"
	"github.com/mattn/go-isatty"
	"github.com/mitchellh/cli"
	"github.com/pkg/errors"
)

const TOPOLOGICAL_PIPELINE_DELMITER = "^"

// RunCommand is a Command implementation that tells Turbo to run a task
type RunCommand struct {
	Ui *cli.ColoredUi

	Config *config.Config
}

// Synopsis of run command
func (c *RunCommand) Synopsis() string {
	return "Run a task"
}

// Help returns information about the `run` command
func (c *RunCommand) Help() string {
	helpText := strings.TrimSpace(`
Usage: turbo run <task> [options] ...

    Run tasks across projects in your monorepo.

    By default, turbo executes tasks in topological order (i.e.
    dependencies first) and then caches the results. Re-running commands for
    tasks already in the cache will skip re-execution and immediately move
    artifacts from the cache into the correct output folders (as if the task
    occurred again).

Options:
  --help                 Show this message.
  --scope                Specify package(s) to act as entry points for task
                         execution. Supports globs.
  --concurrency          Limit the concurrency of task execution. Use 1 for 
                         serial (i.e. one-at-a-time) execution. (default 10)
  --continue             Continue execution even if a task exits with an error
                         or non-zero exit code. The default behavior is to bail
                         immediately. (default false)
  --force                Ignore the existing cache (to force execution). 
                         (default false)
  --graph                Generate a Dot graph of the task execution.   
  --since                Limit/Set scope to changed packages since a
                         mergebase. This uses the git diff ${target_branch}...
                         mechanism to identify which packages have changed.
  --team                 The slug of the turborepo.com team.                         
  --token                A turborepo.com personal access token.
  --ignore               Files to ignore when calculating changed files
                         (i.e. --since). Supports globs.
  --profile              File to write turbo's performance profile output into.
                         You can load the file up in chrome://tracing to see
                         which parts of your build were slow.
  --parallel             Execute all tasks in parallel. (default false)
  --project              The slug of the turborepo.com project.
  --includeDependencies  Include the dependencies of tasks in execution.
                         (default false)
  --no-deps              Exclude affected/dependent task consumers from
                         execution. (default false)
  --no-cache             Avoid saving task results to the cache. Useful for
                         development/watch tasks. (default false)
`)
	return strings.TrimSpace(helpText)
}

// Run executes tasks in the monorepo
func (c *RunCommand) Run(args []string) int {
	startAt := time.Now()
	log.Default()
	log.SetFlags(0)
	flags := flag.NewFlagSet("run", flag.ContinueOnError)
	flags.Usage = func() { c.Config.Logger.Info(c.Help()) }
	if err := flags.Parse(args); err != nil {
		return 1
	}

	cwd, err := os.Getwd()
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("invalid working directory?: %w", err))
		return 1
	}

	runOptions, err := parseRunArgs(args, cwd)
	if err != nil {
		c.logError(c.Config.Logger, "", err)
		return 1
	}

	c.Config.Cache.Dir = runOptions.cacheFolder

	ctx, err := context.New(context.WithTracer(runOptions.profile), context.WithArgs(args), context.WithGraph(".", c.Config))
	if err != nil {
		c.logError(c.Config.Logger, "", err)
		return 1
	}

	gitRepoRoot, err := fs.FindupFrom(".git", cwd)
	if err != nil {
		c.logWarning(c.Config.Logger, "Cannot find a .git folder in current working directory or in any parent directories. Falling back to manual file hashing (which may be slower). If you are running this build in a pruned directory, you can ignore this message. Otherwise, please initialize a git repository in the root of your monorepo.", err)
	}
	git, err := scm.NewFallback(filepath.Dir(gitRepoRoot))
	if err != nil {
		c.logWarning(c.Config.Logger, "", err)
	}

	ignoreGlobs, err := convertStringsToGlobs(runOptions.ignore)
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("invalid ignore globs: %w", err))
		return 1
	}
	globalDeps, err := convertStringsToGlobs(runOptions.globalDeps)
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("invalid global deps: %w", err))
		return 1
	}
	hasRepoGlobalFileChanged := false
	var changedFiles []string
	if runOptions.since != "" {
		changedFilesRelativeToGitRoot := git.ChangedFiles(runOptions.since, true, "")
		// We need to convert relative path of changed files to git root, to relative path to cwd
		for _, f := range changedFilesRelativeToGitRoot {
			repoRoot := filepath.Dir(gitRepoRoot)
			pathToTarget := filepath.Join(repoRoot, f)
			changedFiles = append(changedFiles, strings.Replace(pathToTarget, cwd+"/", "", 1))
		}
	}

	ignoreSet := make(util.Set)

	for _, f := range changedFiles {
		for _, g := range globalDeps {
			if g.Match(f) {
				hasRepoGlobalFileChanged = true
				break
			}
		}
	}

	for _, f := range changedFiles {
		for _, g := range ignoreGlobs {
			if g.Match(f) {
				ignoreSet.Add(f)
			}
		}
	}
	filteredChangedFiles := make(util.Set)
	// Ignore any changed files in the ignore set
	for _, c := range changedFiles {
		if !ignoreSet.Include(c) {
			filteredChangedFiles.Add(c)
		}
	}

	changedPackages := make(util.Set)
	// Be specific with the changed packages only if no repo-wide changes occurred
	if !hasRepoGlobalFileChanged {
		for k, pkgInfo := range ctx.PackageInfos {
			partialPath := pkgInfo.Dir
			if filteredChangedFiles.Some(func(v interface{}) bool {
				return strings.HasPrefix(fmt.Sprintf("%v", v), partialPath) // true
			}) {
				changedPackages.Add(k)
			}
		}
	}

	// Scoped packages
	// Unwind scope globs
	scopePkgs, err := getScopedPackages(ctx, runOptions.scope)

	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("Invalid scope: %w", err))
		return 1
	}

	// Filter Packages
	filteredPkgs := make(util.Set)

	// If both scoped and since are specified, we have to merge two lists:
	// 1. changed packages that ARE themselves the scoped packages
	// 2. changed package consumers (package dependents) that are within the scoped subgraph
	if scopePkgs.Len() > 0 && changedPackages.Len() > 0 {
		filteredPkgs = scopePkgs.Intersection(changedPackages)
		for _, changed := range changedPackages {
			descenders, err := ctx.TopologicalGraph.Descendents(changed)
			if err != nil {
				c.logError(c.Config.Logger, "", fmt.Errorf("could not determine dependency: %w", err))
				return 1
			}

			filteredPkgs.Add(changed)
			for _, d := range descenders {
				filteredPkgs.Add(d)
			}
		}
		c.Ui.Output(fmt.Sprintf(ui.Dim("• Packages changed since %s in scope: %s"), runOptions.since, strings.Join(filteredPkgs.UnsafeListOfStrings(), ", ")))
	} else if changedPackages.Len() > 0 {
		filteredPkgs = changedPackages
		c.Ui.Output(fmt.Sprintf(ui.Dim("• Packages changed since %s: %s"), runOptions.since, strings.Join(filteredPkgs.UnsafeListOfStrings(), ", ")))
	} else if scopePkgs.Len() > 0 {
		filteredPkgs = scopePkgs
		c.Ui.Output(fmt.Sprintf(ui.Dim("• Packages in scope: %v"), strings.Join(scopePkgs.UnsafeListOfStrings(), ", ")))
	} else {
		for _, f := range ctx.PackageNames {
			filteredPkgs.Add(f)
		}
	}

	if runOptions.deps {
		// perf??? this is duplicative from the step above
		for _, changed := range filteredPkgs {
			descenders, err := ctx.TopologicalGraph.Descendents(changed)
			if err != nil {
				c.logError(c.Config.Logger, "", fmt.Errorf("error calculating affected packages: %w", err))
				return 1
			}
			// filteredPkgs.Add(changed)
			for _, d := range descenders {
				filteredPkgs.Add(d)
			}
		}
		c.Config.Logger.Debug("running with dependents")
	}

	if runOptions.ancestors {
		for _, changed := range filteredPkgs {
			ancestors, err := ctx.TopologicalGraph.Ancestors(changed)
			if err != nil {
				log.Printf("error getting dependency %v", err)
				return 1
			}
			c.Config.Logger.Debug("dependencies", ancestors)
			for _, d := range ancestors {
				// we need to exlcude the fake root node
				// since it is not a real package
				if d != ctx.RootNode {
					filteredPkgs.Add(d)
				}
			}
		}
		c.Config.Logger.Debug(ui.Dim("running with dependencies"))
	}
	c.Config.Logger.Debug("execution scope", "packages", strings.Join(filteredPkgs.UnsafeListOfStrings(), ", "))
	c.Config.Logger.Debug("global hash", "value", ctx.GlobalHash)

	c.Config.Logger.Debug("local cache folder", "path", runOptions.cacheFolder)
	fs.EnsureDir(runOptions.cacheFolder)
	turboCache := cache.New(c.Config)
	defer turboCache.Shutdown()
	var topoVisit []interface{}
	for _, node := range ctx.SCC {
		v := node[0]
		if v == ctx.RootNode {
			continue
		}
		topoVisit = append(topoVisit, v)
		pack := ctx.PackageInfos[v]

		ancestralHashes := make([]string, len(pack.InternalDeps))
		if len(pack.InternalDeps) > 0 {
			for i, ancestor := range pack.InternalDeps {
				ancestralHashes[i] = ctx.PackageInfos[ancestor].Hash
			}
		}
		sort.Strings(ancestralHashes)
		var hashable = struct {
			hashOfFiles      string
			ancestralHashes  []string
			externalDepsHash string
			globalHash       string
		}{hashOfFiles: pack.FilesHash, ancestralHashes: ancestralHashes, externalDepsHash: pack.ExternalDepsHash, globalHash: ctx.GlobalHash}

		pack.Hash, err = fs.HashObject(hashable)
		if err != nil {
			log.Printf("[ERROR] %v: error computing combined hash", pack.Name)
		}
		c.Config.Logger.Debug(fmt.Sprintf("%v: package anscestralHash", pack.Name), "hash", ancestralHashes)
		c.Config.Logger.Debug(fmt.Sprintf("%v: package hash", pack.Name), "hash", pack.Hash)
	}

	c.Config.Logger.Debug("topological sort order", "value", topoVisit)

	vertexSet := make(util.Set)
	for _, v := range ctx.TopologicalGraph.Vertices() {
		vertexSet.Add(v)
	}
	// We remove nodes that aren't in the final filter set
	for _, toRemove := range util.Set(vertexSet).Difference(filteredPkgs) {
		if toRemove != ctx.RootNode {
			ctx.TopologicalGraph.Remove(toRemove)
		}
	}

	// If we are running in parallel, then we simply remove all the edges in the graph
	// except for the root
	if runOptions.parallel {
		for _, edge := range ctx.TopologicalGraph.Edges() {
			if edge.Target() != ctx.RootNode {
				ctx.TopologicalGraph.RemoveEdge(edge)
			}
		}
	}

	if runOptions.stream {
		targetList := make([]string, ctx.Targets.Len())
		for i, v := range ctx.Targets.List() {
			targetList[i] = v.(string)
		}
		c.Ui.Output(fmt.Sprintf("%s %s %s", ui.Dim("• Running"), ui.Dim(ui.Bold(strings.Join(targetList, ", "))), ui.Dim(fmt.Sprintf("in %v packages", filteredPkgs.Len()))))
	}
	runState := NewRunState(runOptions)
	runState.Listen(c.Ui, time.Now())
	engine := core.NewScheduler(&ctx.TopologicalGraph)
	var logReplayWaitGroup sync.WaitGroup
	for taskName, value := range ctx.RootPackageJSON.Turbo.Pipeline {
		topoDeps := make(util.Set)
		deps := make(util.Set)
		if core.IsPackageTask(taskName) {
			for _, from := range value.DependsOn {
				if core.IsPackageTask(from) {
					engine.AddDep(from, taskName)
					continue
				} else if strings.Contains(from, TOPOLOGICAL_PIPELINE_DELMITER) {
					topoDeps.Add(from[1:])
				} else {
					deps.Add(from)
				}
			}
			_, id := core.GetPackageTaskFromId(taskName)
			taskName = id
		} else {
			for _, from := range value.DependsOn {
				if strings.Contains(from, TOPOLOGICAL_PIPELINE_DELMITER) {
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
			Cache:    value.Cache,
			Run: func(id string) error {
				cmdTime := time.Now()
				name, task := context.GetPackageTaskFromId(id)
				pack := ctx.PackageInfos[name]
				targetLogger := c.Config.Logger.Named(fmt.Sprintf("%v:%v", pack.Name, task))
				defer targetLogger.ResetNamed(pack.Name)
				targetLogger.Debug("start")

				// bail if the script doesn't exist
				if _, ok := pack.Scripts[task]; !ok {
					targetLogger.Debug("no task in package, skipping")
					targetLogger.Debug("done", "status", "skipped", "duration", time.Since(cmdTime))
					return nil
				}

				// Setup tracer
				tracer := runState.Run(context.GetTaskId(pack.Name, task))

				// Create a logger
				pref := context.PrefixColor(ctx, &pack.Name)
				actualPrefix := pref("%v:%v: ", pack.Name, task)
				targetUi := &cli.PrefixedUi{
					Ui:           c.Ui,
					OutputPrefix: actualPrefix,
					InfoPrefix:   actualPrefix,
					ErrorPrefix:  actualPrefix,
					WarnPrefix:   actualPrefix,
				}
				// Hash ---------------------------------------------
				// first check for package-tasks
				pipeline, ok := ctx.RootPackageJSON.Turbo.Pipeline[fmt.Sprintf("%v", id)]
				if !ok {
					// then check for regular tasks
					altpipe, notcool := ctx.RootPackageJSON.Turbo.Pipeline[task]
					// if neither, then bail
					if !notcool && !ok {
						return nil
					}
					// override if we need to...
					pipeline = altpipe
				}
				hashable := struct {
					Hash         string
					Task         string
					Outputs      []string
					PassThruArgs []string
				}{
					Hash:         pack.Hash,
					Task:         task,
					Outputs:      pipeline.Outputs,
					PassThruArgs: runOptions.passThroughArgs,
				}
				hash, err := fs.HashObject(hashable)
				targetLogger.Debug("task hash", "value", hash)
				if err != nil {
					targetUi.Error(fmt.Sprintf("Hashing error: %v", err))
					// @TODO probably should abort fatally???
				}
				logFileName := path.Join(pack.Dir, ".turbo", fmt.Sprintf("turbo-%v.log", task))
				targetLogger.Debug("log file", "path", path.Join(runOptions.cwd, logFileName))

				// Cache ---------------------------------------------
				// We create the real task outputs now so we can potentially use them to
				// to store artifacts from remote cache to local fs cache

				outputs := []string{fmt.Sprintf(".turbo/turbo-%v.log", task)}
				if len(pipeline.Outputs) > 0 {
					outputs = append(outputs, pipeline.Outputs...)
				} else {
					outputs = append(outputs, "dist/**/*", "build/**/*")
				}

				var hit bool
				if runOptions.forceExecution {
					hit = false
				} else {
					hit, _, err = turboCache.Fetch(pack.Dir, hash, nil)
					if err != nil {
						targetUi.Error(fmt.Sprintf("error fetching from cache: %s", err))
					} else if hit {
						if runOptions.stream && fs.FileExists(path.Join(runOptions.cwd, logFileName)) {
							logReplayWaitGroup.Add(1)
							targetUi.Output(fmt.Sprintf("cache hit, replaying output %s", ui.Dim(hash)))
							go replayLogs(targetLogger, targetUi, runOptions, logFileName, hash, &logReplayWaitGroup, false)
						}
						targetLogger.Debug("done", "status", "complete", "duration", time.Since(cmdTime))
						tracer(TargetCached, nil)
						return nil
					}
				}
				// Setup log file
				fs.EnsureDir(logFileName)
				output, err := os.Create(path.Join(runOptions.cwd, logFileName))
				if err != nil {
					tracer(TargetBuildFailed, err)
					c.logError(targetLogger, actualPrefix, err)
					if runOptions.bail {
						os.Exit(1)
					}
				}
				defer output.Close()
				if runOptions.stream {
					targetUi.Output(fmt.Sprintf("cache miss, executing %s", ui.Dim(hash)))
				}
				argsactual := append([]string{"run"}, task)
				argsactual = append(argsactual, runOptions.passThroughArgs...)
				// @TODO: @jaredpalmer fix this hack to get the package manager's name
				cmd := exec.Command(strings.TrimPrefix(ctx.Backend.Name, "nodejs-"), argsactual...)
				cmd.Dir = pack.Dir
				envs := fmt.Sprintf("TURBO_HASH=%v", hash)
				cmd.Env = append(os.Environ(), envs)

				// Get a pipe to read from stdout and stderr
				stdout, err := cmd.StdoutPipe()
				defer stdout.Close()
				if err != nil {
					tracer(TargetBuildFailed, err)
					c.logError(targetLogger, actualPrefix, err)
					if runOptions.bail {
						os.Exit(1)
					}
				}
				stderr, err := cmd.StderrPipe()
				defer stderr.Close()
				if err != nil {
					tracer(TargetBuildFailed, err)
					c.logError(targetLogger, actualPrefix, err)
					if runOptions.bail {
						os.Exit(1)
					}
				}

				writer := bufio.NewWriter(output)

				// Merge the streams together
				merged := io.MultiReader(stdout, stderr)

				// Create a scanner which scans r in a line-by-line fashion
				scanner := bufio.NewScanner(merged)

				// Execute command
				// Failed to spawn?
				if err := cmd.Start(); err != nil {
					tracer(TargetBuildFailed, err)
					writer.Flush()
					if runOptions.bail {
						targetLogger.Error("Could not spawn command: %w", err)
						targetUi.Error(fmt.Sprintf("Could not spawn command: %v", err))
						os.Exit(1)
					}
					targetUi.Warn("could not spawn command, but continuing...")
				}
				// Read line by line and process it
				if runOptions.stream || runOptions.cache {
					for scanner.Scan() {
						line := scanner.Text()
						if runOptions.stream {
							targetUi.Output(string(scanner.Bytes()))
						}
						if runOptions.cache {
							writer.WriteString(fmt.Sprintf("%v\n", line))
						}
					}
				}

				// Run the command
				if err := cmd.Wait(); err != nil {
					tracer(TargetBuildFailed, err)
					targetLogger.Error("Error: command finished with error: %w", err)
					writer.Flush()
					if runOptions.bail {
						if runOptions.stream {
							targetUi.Error(fmt.Sprintf("Error: command finished with error: %s", err))
							os.Exit(1)
						} else {
							f, err := os.Open(path.Join(runOptions.cwd, logFileName))
							if err != nil {
								targetUi.Warn(fmt.Sprintf("failed reading logs: %v", err))
							}
							defer f.Close()
							scan := bufio.NewScanner(f)
							c.Ui.Error("")
							c.Ui.Error(util.Sprintf("%s ${RED}%s finished with error${RESET}", ui.ERROR_PREFIX, context.GetTaskId(pack.Name, task)))
							c.Ui.Error("")
							for scan.Scan() {
								c.Ui.Output(util.Sprintf("${RED}%s:%s: ${RESET}%s", pack.Name, task, scan.Bytes())) //Writing to Stdout
							}
							os.Exit(1)
						}
					} else {
						if runOptions.stream {
							targetUi.Warn("command finished with error, but continuing...")
						}
					}

					return nil
				}

				writer.Flush()

				if runOptions.cache && (pipeline.Cache == nil || *pipeline.Cache) {
					targetLogger.Debug("caching output", "outputs", outputs)
					var filesToBeCached = make(util.Set)
					for _, output := range outputs {
						results, err := doublestar.Glob(path.Join(pack.Dir, strings.TrimPrefix(output, "!")))
						if err != nil {
							targetUi.Error(fmt.Sprintf("Could not find output artifacts in %v, likely invalid glob %v: %s", pack.Dir, output, err))
						}
						for _, result := range results {
							if strings.HasPrefix(output, "!") {
								filesToBeCached.Delete(result)
							} else {
								filesToBeCached.Add(result)
							}
						}
					}
					if err := turboCache.Put(pack.Dir, hash, filesToBeCached.UnsafeListOfStrings()); err != nil {
						c.logError(targetLogger, "", fmt.Errorf("Error caching output: %w", err))
					}
				}

				tracer(TargetBuilt, nil)
				targetLogger.Debug("done", "status", "complete", "duration", time.Since(cmdTime))
				return nil
			},
		})
	}

	if err := engine.Prepare(&core.SchedulerExecutionOptions{
		Packages:    filteredPkgs.UnsafeListOfStrings(),
		TaskNames:   ctx.Targets.UnsafeListOfStrings(),
		Concurrency: runOptions.concurrency,
		Parallel:    runOptions.parallel,
	}); err != nil {
		c.Ui.Error(fmt.Sprintf("Error preparing engine: %s", err))
		return 1
	}

	if runOptions.dotGraph != "" {
		graphString := string(engine.TaskGraph.Dot(&dag.DotOpts{
			Verbose:    true,
			DrawCycles: true,
		}))
		hasDot := hasGraphViz()
		if hasDot {
			dotArgs := []string{"-T" + path.Ext(runOptions.dotGraph)[1:], "-o", runOptions.dotGraph}
			cmd := exec.Command("dot", dotArgs...)
			cmd.Stdin = strings.NewReader(graphString)
			if err := cmd.Run(); err != nil {
				c.logError(c.Config.Logger, "", fmt.Errorf("could not generate task graphfile %v:  %w", runOptions.dotGraph, err))
				return 1
			} else {
				c.Ui.Output("")
				c.Ui.Output(fmt.Sprintf("✔ Generated task graph in %s", ui.Bold(runOptions.dotGraph)))
			}
		} else {
			c.Ui.Output("")
			c.Ui.Warn(color.New(color.FgYellow, color.Bold, color.ReverseVideo).Sprint(" WARNING ") + color.YellowString(" `turbo` uses Graphviz to generate an image of your\ngraph, but Graphviz isn't installed on this machine.\n\nYou can download Graphviz from https://graphviz.org/download.\n\nIn the meantime, you can use this string output with an\nonline Dot graph viewer."))
			c.Ui.Output("")
			c.Ui.Output(graphString)
		}
		return 0
	}

	// run the thing
	errs := engine.Execute()

	for _, err := range errs {
		c.Ui.Error(err.Error())
	}

	logReplayWaitGroup.Wait()

	if err := runState.Close(c.Ui, startAt, runOptions.profile); err != nil {
		c.Ui.Error(fmt.Sprintf("Error with profiler: %s", err.Error()))
		return 1
	}

	return 0
}

// RunOptions holds the current run operations configuration

type RunOptions struct {
	// Whether to include dependent impacted consumers in execution (defaults to true)
	deps bool
	// Whether to include ancestors (pkg.dependencies) in execution (defaults to false)
	ancestors bool
	// List of globs of file paths to ignore from exection scope calculation
	ignore []string
	// Whether to stream log outputs
	stream bool
	// Show a dot graph
	dotGraph string
	// List of globs to global files whose contents will be included in the global hash calculation
	globalDeps []string
	// Filtered list of package entrypoints
	scope []string
	// Force execution to be serially one-at-a-time
	concurrency int
	// Whether to execute in parallel (defaults to false)
	parallel bool
	// Git diff used to calculate changed packages
	since string
	// Current working directory
	cwd string
	// Whether to emit a perf profile
	profile string
	// Force task execution
	forceExecution bool
	// Cache results
	cache bool
	// Cache folder
	cacheFolder string
	// Immediately exit on task failure
	bail            bool
	passThroughArgs []string
}

func getDefaultRunOptions() *RunOptions {
	return &RunOptions{
		bail:           true,
		deps:           true,
		parallel:       false,
		concurrency:    10,
		dotGraph:       "",
		ancestors:      false,
		cache:          true,
		profile:        "", // empty string does no tracing
		forceExecution: false,
		stream:         true,
	}
}

func parseRunArgs(args []string, cwd string) (*RunOptions, error) {
	var runOptions = getDefaultRunOptions()

	if len(args) == 0 {
		return nil, errors.Errorf("At least one task must be specified.")
	}

	unresolvedCacheFolder := "./node_modules/.cache/turbo"

	for argIndex, arg := range args {
		if arg == "--" {
			runOptions.passThroughArgs = args[argIndex+1:]
			break
		} else if strings.HasPrefix(arg, "--") {
			switch {
			case strings.HasPrefix(arg, "--since="):
				if len(arg[len("--since="):]) > 1 {
					runOptions.since = arg[len("--since="):]
				}
			case strings.HasPrefix(arg, "--scope="):
				if len(arg[len("--scope="):]) > 1 {
					runOptions.scope = append(runOptions.scope, arg[len("--scope="):])
				}
			case strings.HasPrefix(arg, "--ignore="):
				if len(arg[len("--ignore="):]) > 1 {
					runOptions.ignore = append(runOptions.ignore, arg[len("--ignore="):])
				}
			case strings.HasPrefix(arg, "--global-deps="):
				if len(arg[len("--global-deps="):]) > 1 {
					runOptions.globalDeps = append(runOptions.ignore, arg[len("--global-deps="):])
				}
			case strings.HasPrefix(arg, "--cwd="):
				if len(arg[len("--cwd="):]) > 1 {
					runOptions.cwd = arg[len("--cwd="):]
				} else {
					runOptions.cwd = cwd
				}
			case strings.HasPrefix(arg, "--parallel"):
				runOptions.parallel = true
			case strings.HasPrefix(arg, "--profile="): // this one must com before the next
				if len(arg[len("--profile="):]) > 1 {
					runOptions.profile = arg[len("--profile="):]
				}
			case strings.HasPrefix(arg, "--profile"):
				runOptions.profile = fmt.Sprintf("%v-profile.json", time.Now().UnixNano())

			case strings.HasPrefix(arg, "--no-deps"):
				runOptions.deps = false
			case strings.HasPrefix(arg, "--no-cache"):
				runOptions.cache = true
			case strings.HasPrefix(arg, "--cacheFolder"):
				unresolvedCacheFolder = arg[len("--cacheFolder="):]
			case strings.HasPrefix(arg, "--continue"):
				runOptions.bail = false
			case strings.HasPrefix(arg, "--force"):
				runOptions.forceExecution = true
			case strings.HasPrefix(arg, "--stream"):
				runOptions.stream = true

			case strings.HasPrefix(arg, "--graph="): // this one must com before the next
				if len(arg[len("--graph="):]) > 1 {
					runOptions.dotGraph = arg[len("--graph="):]
				}
			case strings.HasPrefix(arg, "--graph"):
				runOptions.dotGraph = fmt.Sprintf("graph-%v.jpg", time.Now().UnixNano())
			case strings.HasPrefix(arg, "--serial"):
				log.Printf("[WARNING] The --serial flag has been deprecated and will be removed in future versions of turbo. Please use `--concurrency=1` instead")
				runOptions.concurrency = 1
			case strings.HasPrefix(arg, "--concurrency"):
				if i, err := strconv.Atoi(arg[len("--concurrency="):]); err != nil {
					return nil, fmt.Errorf("Invalid value for --concurrency CLI flag. This should be a positive integer greater than or equal to 1: %w", err)
				} else {
					if i >= 1 {
						runOptions.concurrency = i
					} else {
						return nil, fmt.Errorf("Invalid value %v for --concurrency CLI flag. This should be a positive integer greater than or equal to 1.", i)
					}
				}
			case strings.HasPrefix(arg, "--includeDependencies"):
				runOptions.ancestors = true
			case strings.HasPrefix(arg, "--team"):
			case strings.HasPrefix(arg, "--project"):
			case strings.HasPrefix(arg, "--token"):
			default:
				return nil, errors.New(fmt.Sprintf("unknown flag: %v", arg))
			}
		}
	}

	// Force streaming output in CI/CD non-interactive mode
	if !isatty.IsTerminal(os.Stdout.Fd()) || os.Getenv("CI") != "" {
		runOptions.stream = true
	}

	// We can only set this cache folder after we know actual cwd
	runOptions.cacheFolder = path.Join(runOptions.cwd, unresolvedCacheFolder)

	return runOptions, nil
}

// convertStringsToGlobs converts string glob patterns to an array glob.Glob instances.
func convertStringsToGlobs(patterns []string) (globss []glob.Glob, err error) {
	var globs = make([]glob.Glob, len(patterns))
	for i, pattern := range patterns {
		g, err := glob.Compile(pattern)
		if err != nil {
			return nil, err
		}
		globs[i] = g
	}

	return globs, nil
}

// getScopedPackages returns a set of package names in scope for a given list of glob patterns
func getScopedPackages(ctx *context.Context, scopePatterns []string) (scopePkgs util.Set, err error) {
	scopeGlobs, err := convertStringsToGlobs(scopePatterns)
	if err != nil {
		return nil, fmt.Errorf("invalid glob pattern %w", err)
	}
	var scopedPkgs = make(util.Set)
	for _, f := range ctx.PackageNames {
		for _, g := range scopeGlobs {
			if g.Match(f) {
				scopedPkgs.Add(f)
			}
		}
	}

	return scopedPkgs, nil
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

// logError logs an error and outputs it to the UI.
func (c *RunCommand) logFatal(log hclog.Logger, prefix string, err error) {
	log.Error(prefix, "error", err)

	if prefix != "" {
		prefix += ": "
	}

	c.Ui.Error(fmt.Sprintf("%s%s%s", ui.ERROR_PREFIX, prefix, color.RedString(" %v", err)))
	os.Exit(1)
}

func hasGraphViz() bool {
	err := exec.Command("dot", "-v").Run()
	return err == nil
}

// Replay logs will try to replay logs back to the stdout
func replayLogs(logger hclog.Logger, prefixUi cli.Ui, runOptions *RunOptions, logFileName, hash string, wg *sync.WaitGroup, silent bool) {
	defer wg.Done()
	logger.Debug("start replaying logs")
	f, err := os.Open(path.Join(runOptions.cwd, logFileName))
	if err != nil && !silent {
		prefixUi.Warn(fmt.Sprintf("error reading logs: %v", err))
		logger.Error(fmt.Sprintf("error reading logs: %v", err.Error()))
	}
	defer f.Close()
	scan := bufio.NewScanner(f)
	for scan.Scan() {
		prefixUi.Output(ui.Dim(string(scan.Bytes()))) //Writing to Stdout
	}
	logger.Debug("finish replaying logs")
}
