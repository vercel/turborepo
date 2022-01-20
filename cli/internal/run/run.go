package run

import (
	"bufio"
	"flag"
	"fmt"
	"io"
	"log"
	"os"
	"os/exec"
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
	"turbo/internal/globby"
	"turbo/internal/logstreamer"
	"turbo/internal/scm"
	"turbo/internal/ui"
	"turbo/internal/util"
	"turbo/internal/util/browser"
	"turbo/internal/util/filter"

	"github.com/pyr-sh/dag"

	"github.com/fatih/color"
	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
	"github.com/pkg/errors"
)

const TOPOLOGICAL_PIPELINE_DELMITER = "^"
const ENV_PIPELINE_DELMITER = "$"

// RunCommand is a Command implementation that tells Turbo to run a task
type RunCommand struct {
	Config *config.Config
	Ui     *cli.ColoredUi
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
  --cache-dir            Specify local filesystem cache directory.
												 (default "./node_modules/.cache/turbo")
  --concurrency          Limit the concurrency of task execution. Use 1 for 
                         serial (i.e. one-at-a-time) execution. (default 10)
  --continue             Continue execution even if a task exits with an error
                         or non-zero exit code. The default behavior is to bail
                         immediately. (default false)
  --force                Ignore the existing cache (to force execution). 
                         (default false)
  --graph                Generate a Dot graph of the task execution.   
  --global-deps          Specify glob of global filesystem dependencies to 
	                       be hashed. Useful for .env and files in the root
												 directory. Can be specified multiple times.
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
  --include-dependencies Include the dependencies of tasks in execution.
                         (default false)
  --no-deps              Exclude dependent task consumers from execution.
                         (default false)
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

	ignoreGlob, err := filter.Compile(runOptions.ignore)
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("invalid ignore globs: %w", err))
		return 1
	}
	globalDepsGlob, err := filter.Compile(runOptions.globalDeps)
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("invalid global deps glob: %w", err))
		return 1
	}
	hasRepoGlobalFileChanged := false
	var changedFiles []string
	if runOptions.since != "" {
		changedFiles = git.ChangedFiles(runOptions.since, true, cwd)
	}

	ignoreSet := make(util.Set)
	if globalDepsGlob != nil {
		for _, f := range changedFiles {
			if globalDepsGlob.Match(f) {
				hasRepoGlobalFileChanged = true
				break
			}
		}
	}

	if ignoreGlob != nil {
		for _, f := range changedFiles {
			if ignoreGlob.Match(f) {
				ignoreSet.Add(f)
			}
		}
	}

	filteredChangedFiles := make(util.Set)
	// Ignore any changed files in the ignore set
	for _, c := range changedFiles {
		if !ignoreSet.Includes(c) {
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
		c.logError(c.Config.Logger, "", fmt.Errorf("invalid scope: %w", err))
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
	} else {
		for _, f := range ctx.PackageNames {
			filteredPkgs.Add(f)
		}
	}

	if runOptions.includeDependents {
		// perf??? this is duplicative from the step above
		for _, pkg := range filteredPkgs {
			descenders, err := ctx.TopologicalGraph.Descendents(pkg)
			if err != nil {
				c.logError(c.Config.Logger, "", fmt.Errorf("error calculating affected packages: %w", err))
				return 1
			}
			c.Config.Logger.Debug("dependents", "pkg", pkg, "value", descenders.List())
			for _, d := range descenders {
				// we need to exlcude the fake root node
				// since it is not a real package
				if d != ctx.RootNode {
					filteredPkgs.Add(d)
				}
			}
		}
		c.Config.Logger.Debug("running with dependents")
	}

	if runOptions.includeDependencies {
		for _, pkg := range filteredPkgs {
			ancestors, err := ctx.TopologicalGraph.Ancestors(pkg)
			if err != nil {
				log.Printf("error getting dependency %v", err)
				return 1
			}
			c.Config.Logger.Debug("dependencies", "pkg", pkg, "value", ancestors.List())
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
	c.Config.Logger.Debug("global hash", "value", ctx.GlobalHash)
	packagesInScope := filteredPkgs.UnsafeListOfStrings()
	sort.Strings(packagesInScope)
	c.Ui.Output(fmt.Sprintf(ui.Dim("• Packages in scope: %v"), strings.Join(packagesInScope, ", ")))
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

		ancestralHashes := make([]string, 0, len(pack.InternalDeps))
		if len(pack.InternalDeps) > 0 {
			for _, ancestor := range pack.InternalDeps {
				if h, ok := ctx.PackageInfos[ancestor]; ok {
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
		}{hashOfFiles: pack.FilesHash, ancestralHashes: ancestralHashes, externalDepsHash: pack.ExternalDepsHash, globalHash: ctx.GlobalHash}

		pack.Hash, err = fs.HashObject(hashable)
		if err != nil {
			log.Printf("[ERROR] %v: error computing combined hash", pack.Name)
		}
		c.Config.Logger.Debug(fmt.Sprintf("%v: package ancestralHash", pack.Name), "hash", ancestralHashes)
		c.Config.Logger.Debug(fmt.Sprintf("%v: package hash", pack.Name), "hash", pack.Hash)
	}

	c.Config.Logger.Debug("topological sort order", "value", topoVisit)

	vertexSet := make(util.Set)
	for _, v := range ctx.TopologicalGraph.Vertices() {
		vertexSet.Add(v)
	}
	// We remove nodes that aren't in the final filter set
	for _, toRemove := range vertexSet.Difference(filteredPkgs) {
		if toRemove != ctx.RootNode {
			ctx.TopologicalGraph.Remove(toRemove)
		}
	}

	// If we are running in parallel, then we remove all the edges in the graph
	// except for the root
	if runOptions.parallel {
		for _, edge := range ctx.TopologicalGraph.Edges() {
			if edge.Target() != ctx.RootNode {
				ctx.TopologicalGraph.RemoveEdge(edge)
			}
		}
	}

	if runOptions.stream {
		c.Ui.Output(fmt.Sprintf("%s %s %s", ui.Dim("• Running"), ui.Dim(ui.Bold(strings.Join(ctx.Targets, ", "))), ui.Dim(fmt.Sprintf("in %v packages", filteredPkgs.Len()))))
	}
	runState := NewRunState(runOptions)
	runState.Listen(c.Ui, time.Now())
	engine := core.NewScheduler(&ctx.TopologicalGraph)
	var logReplayWaitGroup sync.WaitGroup
	for taskName, value := range ctx.RootPackageJSON.Turbo.Pipeline {
		topoDeps := make(util.Set)
		deps := make(util.Set)
		if util.IsPackageTask(taskName) {
			for _, from := range value.DependsOn {
				if strings.HasPrefix(from, ENV_PIPELINE_DELMITER) {
					continue
				}
				if util.IsPackageTask(from) {
					engine.AddDep(from, taskName)
					continue
				} else if strings.Contains(from, TOPOLOGICAL_PIPELINE_DELMITER) {
					topoDeps.Add(from[1:])
				} else {
					deps.Add(from)
				}
			}
			_, id := util.GetPackageTaskFromId(taskName)
			taskName = id
		} else {
			for _, from := range value.DependsOn {
				if strings.HasPrefix(from, ENV_PIPELINE_DELMITER) {
					continue
				}
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
				name, task := util.GetPackageTaskFromId(id)
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
				tracer := runState.Run(util.GetTaskId(pack.Name, task))

				// Create a logger
				pref := ctx.ColorCache.PrefixColor(pack.Name)
				actualPrefix := pref("%s:%s: ", pack.Name, task)
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

				outputs := []string{fmt.Sprintf(".turbo/turbo-%v.log", task)}
				if pipeline.Outputs == nil {
					outputs = append(outputs, "dist/**/*", "build/**/*")
				} else {
					outputs = append(outputs, pipeline.Outputs...)
				}
				targetLogger.Debug("task output globs", "outputs", outputs)

				// Hash the task-specific environment variables found in the dependsOnKey in the pipeline
				var hashabledEnvVars []string
				var hashabledEnvPairs []string
				if len(pipeline.DependsOn) > 0 {
					for _, v := range pipeline.DependsOn {
						if strings.Contains(v, ENV_PIPELINE_DELMITER) {
							trimmed := strings.TrimPrefix(v, ENV_PIPELINE_DELMITER)
							hashabledEnvPairs = append(hashabledEnvPairs, fmt.Sprintf("%v=%v", trimmed, os.Getenv(trimmed)))
							hashabledEnvVars = append(hashabledEnvVars, trimmed)
						}
					}
					sort.Strings(hashabledEnvVars) // always sort them
				}
				targetLogger.Debug("hashable env vars", "vars", hashabledEnvVars)
				hashable := struct {
					Hash             string
					Task             string
					Outputs          []string
					PassThruArgs     []string
					HashableEnvPairs []string
				}{
					Hash:             pack.Hash,
					Task:             task,
					Outputs:          outputs,
					PassThruArgs:     runOptions.passThroughArgs,
					HashableEnvPairs: hashabledEnvPairs,
				}
				hash, err := fs.HashObject(hashable)
				targetLogger.Debug("task hash", "value", hash)
				if err != nil {
					targetUi.Error(fmt.Sprintf("Hashing error: %v", err))
					// @TODO probably should abort fatally???
				}
				logFileName := filepath.Join(pack.Dir, ".turbo", fmt.Sprintf("turbo-%v.log", task))
				targetLogger.Debug("log file", "path", filepath.Join(runOptions.cwd, logFileName))

				// Cache ---------------------------------------------
				var hit bool
				if !runOptions.forceExecution {
					hit, _, err = turboCache.Fetch(pack.Dir, hash, nil)
					if err != nil {
						targetUi.Error(fmt.Sprintf("error fetching from cache: %s", err))
					} else if hit {
						if runOptions.stream && fs.FileExists(filepath.Join(runOptions.cwd, logFileName)) {
							logReplayWaitGroup.Add(1)
							go replayLogs(targetLogger, c.Ui, runOptions, logFileName, hash, &logReplayWaitGroup, false)
						}
						targetLogger.Debug("done", "status", "complete", "duration", time.Since(cmdTime))
						tracer(TargetCached, nil)

						return nil
					}
					if runOptions.stream {
						targetUi.Output(fmt.Sprintf("cache miss, executing %s", ui.Dim(hash)))
					}
				} else {
					if runOptions.stream {
						targetUi.Output(fmt.Sprintf("cache bypass, force executing %s", ui.Dim(hash)))
					}
				}

				// Setup command execution
				argsactual := append([]string{"run"}, task)
				argsactual = append(argsactual, runOptions.passThroughArgs...)
				// @TODO: @jaredpalmer fix this hack to get the package manager's name
				var cmd *exec.Cmd
				if ctx.Backend.Name == "nodejs-berry" {
					cmd = exec.Command("yarn", argsactual...)
				} else {
					cmd = exec.Command(strings.TrimPrefix(ctx.Backend.Name, "nodejs-"), argsactual...)
				}
				cmd.Dir = pack.Dir
				envs := fmt.Sprintf("TURBO_HASH=%v", hash)
				cmd.Env = append(os.Environ(), envs)

				// Setup stdout/stderr
				// If we are not caching anything, then we don't need to write logs to disk
				// be careful about this conditional given the default of cache = true
				var writer io.Writer
				if !runOptions.cache || (pipeline.Cache != nil && !*pipeline.Cache) {
					writer = os.Stdout
				} else {
					// Setup log file
					if err := fs.EnsureDir(logFileName); err != nil {
						tracer(TargetBuildFailed, err)
						c.logError(targetLogger, actualPrefix, err)
						if runOptions.bail {
							os.Exit(1)
						}
					}
					output, err := os.Create(logFileName)
					if err != nil {
						tracer(TargetBuildFailed, err)
						c.logError(targetLogger, actualPrefix, err)
						if runOptions.bail {
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
				if err := cmd.Run(); err != nil {
					tracer(TargetBuildFailed, err)
					targetLogger.Error("Error: command finished with error: %w", err)
					if runOptions.bail {
						if runOptions.stream {
							targetUi.Error(fmt.Sprintf("Error: command finished with error: %s", err))
							os.Exit(1)
						} else {
							f, err := os.Open(filepath.Join(runOptions.cwd, logFileName))
							if err != nil {
								targetUi.Warn(fmt.Sprintf("failed reading logs: %v", err))
							}
							defer f.Close()
							scan := bufio.NewScanner(f)
							c.Ui.Error("")
							c.Ui.Error(util.Sprintf("%s ${RED}%s finished with error${RESET}", ui.ERROR_PREFIX, util.GetTaskId(pack.Name, task)))
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

				// Cache command outputs
				if runOptions.cache && (pipeline.Cache == nil || *pipeline.Cache) {
					targetLogger.Debug("caching output", "outputs", outputs)
					ignore := []string{}
					filesToBeCached := globby.GlobFiles(pack.Dir, outputs, ignore)
					if err := turboCache.Put(pack.Dir, hash, int(time.Since(cmdTime).Milliseconds()), filesToBeCached); err != nil {
						c.logError(targetLogger, "", fmt.Errorf("error caching output: %w", err))
					}
				}

				// Clean up tracing
				tracer(TargetBuilt, nil)
				targetLogger.Debug("done", "status", "complete", "duration", time.Since(cmdTime))
				return nil
			},
		})
	}

	if err := engine.Prepare(&core.SchedulerExecutionOptions{
		Packages:    filteredPkgs.UnsafeListOfStrings(),
		TaskNames:   ctx.Targets,
		Concurrency: runOptions.concurrency,
		Parallel:    runOptions.parallel,
		TasksOnly:   runOptions.only,
	}); err != nil {
		c.Ui.Error(fmt.Sprintf("Error preparing engine: %s", err))
		return 1
	}

	if runOptions.dotGraph != "" {
		graphString := string(engine.TaskGraph.Dot(&dag.DotOpts{
			Verbose:    true,
			DrawCycles: true,
		}))
		ext := filepath.Ext(runOptions.dotGraph)
		if ext == ".html" {
			f, err := os.Create(filepath.Join(cwd, runOptions.dotGraph))
			if err != nil {
				c.logError(c.Config.Logger, "", fmt.Errorf("error writing graph: %w", err))
				return 1
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
			c.Ui.Output(fmt.Sprintf("✔ Generated task graph in %s", ui.Bold(runOptions.dotGraph)))
			if ui.IsTTY {
				browser.OpenBrowser(filepath.Join(cwd, runOptions.dotGraph))
			}
			return 0
		}
		hasDot := hasGraphViz()
		if hasDot {
			dotArgs := []string{"-T" + ext[1:], "-o", runOptions.dotGraph}
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
	includeDependents bool
	// Whether to include includeDependencies (pkg.dependencies) in execution (defaults to false)
	includeDependencies bool
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
	// Restrict execution to only the listed task names. Default false
	only bool
}

func getDefaultRunOptions() *RunOptions {
	return &RunOptions{
		bail:                true,
		includeDependents:   true,
		parallel:            false,
		concurrency:         10,
		dotGraph:            "",
		includeDependencies: false,
		cache:               true,
		profile:             "", // empty string does no tracing
		forceExecution:      false,
		stream:              true,
		only:                false,
	}
}

func parseRunArgs(args []string, cwd string) (*RunOptions, error) {
	var runOptions = getDefaultRunOptions()

	if len(args) == 0 {
		return nil, errors.Errorf("At least one task must be specified.")
	}

	unresolvedCacheFolder := filepath.FromSlash("./node_modules/.cache/turbo")

	for argIndex, arg := range args {
		if arg == "--" {
			runOptions.passThroughArgs = args[argIndex+1:]
			break
		} else if strings.HasPrefix(arg, "--") {
			switch {
			case strings.HasPrefix(arg, "--since="):
				if len(arg[len("--since="):]) > 0 {
					runOptions.since = arg[len("--since="):]
				}
			case strings.HasPrefix(arg, "--scope="):
				if len(arg[len("--scope="):]) > 0 {
					runOptions.scope = append(runOptions.scope, arg[len("--scope="):])
				}
			case strings.HasPrefix(arg, "--ignore="):
				if len(arg[len("--ignore="):]) > 0 {
					runOptions.ignore = append(runOptions.ignore, arg[len("--ignore="):])
				}
			case strings.HasPrefix(arg, "--global-deps="):
				if len(arg[len("--global-deps="):]) > 0 {
					runOptions.globalDeps = append(runOptions.globalDeps, arg[len("--global-deps="):])
				}
			case strings.HasPrefix(arg, "--cwd="):
				if len(arg[len("--cwd="):]) > 0 {
					runOptions.cwd = arg[len("--cwd="):]
				} else {
					runOptions.cwd = cwd
				}
			case strings.HasPrefix(arg, "--parallel"):
				runOptions.parallel = true
			case strings.HasPrefix(arg, "--profile="): // this one must com before the next
				if len(arg[len("--profile="):]) > 0 {
					runOptions.profile = arg[len("--profile="):]
				}
			case strings.HasPrefix(arg, "--profile"):
				runOptions.profile = fmt.Sprintf("%v-profile.json", time.Now().UnixNano())

			case strings.HasPrefix(arg, "--no-deps"):
				runOptions.includeDependents = false
			case strings.HasPrefix(arg, "--no-cache"):
				runOptions.cache = true
			case strings.HasPrefix(arg, "--cacheFolder"):
				log.Printf("[WARNING] The --cacheFolder flag has been deprecated and will be removed in future versions of turbo. Please use `--cache-dir` instead")
				unresolvedCacheFolder = arg[len("--cacheFolder="):]
			case strings.HasPrefix(arg, "--cache-dir"):
				unresolvedCacheFolder = arg[len("--cache-dir="):]
			case strings.HasPrefix(arg, "--continue"):
				runOptions.bail = false
			case strings.HasPrefix(arg, "--force"):
				runOptions.forceExecution = true
			case strings.HasPrefix(arg, "--stream"):
				runOptions.stream = true

			case strings.HasPrefix(arg, "--graph="): // this one must com before the next
				if len(arg[len("--graph="):]) > 0 {
					runOptions.dotGraph = arg[len("--graph="):]
				}
			case strings.HasPrefix(arg, "--graph"):
				runOptions.dotGraph = fmt.Sprintf("graph-%v.jpg", time.Now().UnixNano())
			case strings.HasPrefix(arg, "--serial"):
				log.Printf("[WARNING] The --serial flag has been deprecated and will be removed in future versions of turbo. Please use `--concurrency=1` instead")
				runOptions.concurrency = 1
			case strings.HasPrefix(arg, "--concurrency"):
				if i, err := strconv.Atoi(arg[len("--concurrency="):]); err != nil {
					return nil, fmt.Errorf("invalid value for --concurrency CLI flag. This should be a positive integer greater than or equal to 1: %w", err)
				} else {
					if i >= 1 {
						runOptions.concurrency = i
					} else {
						return nil, fmt.Errorf("invalid value %v for --concurrency CLI flag. This should be a positive integer greater than or equal to 1", i)
					}
				}
			case strings.HasPrefix(arg, "--includeDependencies"):
				log.Printf("[WARNING] The --includeDependencies flag has renamed to --include-dependencies for consistency. Please use `--include-dependencies` instead")
				runOptions.includeDependencies = true
			case strings.HasPrefix(arg, "--include-dependencies"):
				runOptions.includeDependencies = true
			case strings.HasPrefix(arg, "--only"):
				runOptions.only = true
			case strings.HasPrefix(arg, "--team"):
			case strings.HasPrefix(arg, "--token"):
			case strings.HasPrefix(arg, "--api"):
			case strings.HasPrefix(arg, "--url"):
			case strings.HasPrefix(arg, "--trace"):
			case strings.HasPrefix(arg, "--cpuprofile"):
			case strings.HasPrefix(arg, "--heap"):
			case strings.HasPrefix(arg, "--no-gc"):
			default:
				return nil, errors.New(fmt.Sprintf("unknown flag: %v", arg))
			}
		}
	}

	// Force streaming output in CI/CD non-interactive mode
	if !ui.IsTTY || ui.IsCI {
		runOptions.stream = true
	}

	// We can only set this cache folder after we know actual cwd
	runOptions.cacheFolder = filepath.Join(runOptions.cwd, unresolvedCacheFolder)

	return runOptions, nil
}

// getScopedPackages returns a set of package names in scope for a given list of glob patterns
func getScopedPackages(ctx *context.Context, scopePatterns []string) (scopePkgs util.Set, err error) {
	if err != nil {
		return nil, fmt.Errorf("invalid glob pattern %w", err)
	}
	var scopedPkgs = make(util.Set)
	if len(scopePatterns) == 0 {
		return scopePkgs, nil
	}
	glob, err := filter.Compile(scopePatterns)
	if err != nil {
		return nil, err
	}
	for _, f := range ctx.PackageNames {
		if glob.Match(f) {
			scopedPkgs.Add(f)
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

func hasGraphViz() bool {
	err := exec.Command("dot", "-v").Run()
	return err == nil
}

// Replay logs will try to replay logs back to the stdout
func replayLogs(logger hclog.Logger, prefixUi cli.Ui, runOptions *RunOptions, logFileName, hash string, wg *sync.WaitGroup, silent bool) {
	defer wg.Done()
	logger.Debug("start replaying logs")
	f, err := os.Open(filepath.Join(runOptions.cwd, logFileName))
	if err != nil && !silent {
		prefixUi.Warn(fmt.Sprintf("error reading logs: %v", err))
		logger.Error(fmt.Sprintf("error reading logs: %v", err.Error()))
	}
	defer f.Close()
	scan := bufio.NewScanner(f)
	for scan.Scan() {
		prefixUi.Output(ui.StripAnsi(string(scan.Bytes()))) //Writing to Stdout
	}
	logger.Debug("finish replaying logs")
}
