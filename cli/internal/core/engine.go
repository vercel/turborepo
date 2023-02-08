package core

import (
	"fmt"
	"sort"
	"strings"

	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/graph"
	"github.com/vercel/turbo/cli/internal/util"

	"github.com/pyr-sh/dag"
)

const ROOT_NODE_NAME = "___ROOT___"

// Task is a higher level struct that contains the underlying TaskDefinition
// but also some adjustments to it, based on business logic.
type Task struct {
	Name string
	// TaskDefinition contains the config for the task from turbo.json
	TaskDefinition fs.TaskDefinition
}

type Visitor = func(taskID string) error

// Engine contains both the DAG for the packages and the tasks and implements the methods to execute tasks in them
type Engine struct {
	// TaskGraph is a graph of package-tasks
	TaskGraph        *dag.AcyclicGraph
	PackageTaskDeps  map[string][]string
	rootEnabledTasks util.Set

	// completeGraph is the CompleteGraph. We need this to look up the Pipeline, etc.
	completeGraph *graph.CompleteGraph

	// Map of packageName to pipeline. We resolve task definitions from here
	// but we don't want to read from the filesystem every time
	pipelines map[string]fs.Pipeline

	// isSinglePackage is used to load turbo.json correctly
	isSinglePackage bool
}

// NewEngine creates a new engine given a topologic graph of workspace package names
func NewEngine(
	completeGraph *graph.CompleteGraph,
	isSinglePackage bool,
) *Engine {
	return &Engine{
		completeGraph:    completeGraph,
		TaskGraph:        &dag.AcyclicGraph{},
		PackageTaskDeps:  map[string][]string{},
		rootEnabledTasks: make(util.Set),
		pipelines:        map[string]fs.Pipeline{},
		isSinglePackage:  isSinglePackage,
	}
}

// EngineBuildingOptions help construct the TaskGraph
type EngineBuildingOptions struct {
	// Packages in the execution scope, if nil, all packages will be considered in scope
	Packages []string
	// TaskNames in the execution scope, if nil, all tasks will be executed
	TaskNames []string
	// Restrict execution to only the listed task names
	TasksOnly bool
}

// EngineExecutionOptions controls a single walk of the task graph
type EngineExecutionOptions struct {
	// Parallel is whether to run tasks in parallel
	Parallel bool
	// Concurrency is the number of concurrent tasks that can be executed
	Concurrency int
}

// Execute executes the pipeline, constructing an internal task graph and walking it accordingly.
func (e *Engine) Execute(visitor Visitor, opts EngineExecutionOptions) []error {
	var sema = util.NewSemaphore(opts.Concurrency)
	return e.TaskGraph.Walk(func(v dag.Vertex) error {
		// Each vertex in the graph is a taskID (package#task format)
		taskID := dag.VertexName(v)

		// Always return if it is the root node
		if strings.Contains(taskID, ROOT_NODE_NAME) {
			return nil
		}

		// Acquire the semaphore unless parallel
		if !opts.Parallel {
			sema.Acquire()
			defer sema.Release()
		}

		return visitor(taskID)
	})
}

func (e *Engine) getTaskDefinition(pkg string, taskName string, taskID string) (*Task, error) {
	pipeline, err := e.getPipelineFromWorkspace(pkg)

	if err != nil {
		// Fallback to the root workspace to look for a pipeline if one wasn't
		// found in the target workspace.
		if pkg != util.RootPkgName {
			return e.getTaskDefinition(util.RootPkgName, taskName, taskID)
		}

		return nil, err
	}

	if task, ok := pipeline[taskID]; ok {
		return &Task{
			Name:           taskName,
			TaskDefinition: task.TaskDefinition,
		}, nil
	}

	if task, ok := pipeline[taskName]; ok {
		return &Task{
			Name:           taskName,
			TaskDefinition: task.TaskDefinition,
		}, nil
	}

	return nil, fmt.Errorf("Could not find \"%s\" or \"%s\" in root config", taskName, taskID)
}

// Prepare constructs the Task Graph for a list of packages and tasks
func (e *Engine) Prepare(options *EngineBuildingOptions) error {
	pkgs := options.Packages
	taskNames := options.TaskNames
	tasksOnly := options.TasksOnly

	traversalQueue := []string{}

	// Get a list of entry points into our TaskGraph.
	// We do this by taking the input taskNames, and pkgs
	// and creating a queue of taskIDs that we can traverse and gather dependencies from.
	for _, pkg := range pkgs {
		isRootPkg := pkg == util.RootPkgName

		for _, taskName := range taskNames {
			// If it's not a task from the root workspace (i.e. tasks from every other workspace)
			// or if it's a task that we know is rootEnabled task, add it to the traversal queue.
			if !isRootPkg || e.rootEnabledTasks.Includes(taskName) {
				taskID := util.GetTaskId(pkg, taskName)
				// Skip tasks that don't have a definition
				if _, err := e.getTaskDefinition(pkg, taskName, taskID); err != nil {
					// Initially, non-package tasks are not required to exist, as long as some
					// package in the list packages defines it as a package-task. Dependencies
					// *are* required to have a definition.
					continue
				}

				traversalQueue = append(traversalQueue, taskID)
			}
		}
	}

	visited := make(util.Set)

	// Things get appended to traversalQueue inside this loop, so we use the len() check instead of range.
	for len(traversalQueue) > 0 {
		// pop off the first item from the traversalQueue
		taskID := traversalQueue[0]
		traversalQueue = traversalQueue[1:]

		pkg, taskName := util.GetPackageTaskFromId(taskID)

		if pkg == util.RootPkgName && !e.rootEnabledTasks.Includes(taskName) {
			return fmt.Errorf("%v needs an entry in turbo.json before it can be depended on because it is a task run from the root package", taskID)
		}

		var pkgDefinition *fs.PackageJSON
		if pkg == ROOT_NODE_NAME {
			pkgDefinition = &fs.PackageJSON{}
		} else {
			pkgJSON, ok := e.completeGraph.WorkspaceInfos[pkg]
			if !ok {
				// If we have a pkg it should be in WorkspaceInfos.
				// If we're hitting this error something has gone wrong earlier when building WorkspaceInfos
				// or the workspace really doesn't exist and turbo.json is misconfigured.
				return fmt.Errorf("Could not find task \"%s\" in project", taskID)
			}

			pkgDefinition = pkgJSON
		}

		taskDefinitions, err := e.getTaskDefinitionChain(&e.completeGraph.Pipeline, pkgDefinition, taskID, taskName)
		if err != nil {
			return err
		}

		taskDefinition, err := fs.MergeTaskDefinitions(taskDefinitions)
		if err != nil {
			return err
		}

		// Skip this iteration of the loop if we've already seen this taskID
		if visited.Includes(taskID) {
			continue
		}

		visited.Add(taskID)

		// Put this taskDefinition into the Graph so we can look it up later during execution.
		e.completeGraph.TaskDefinitions[taskID] = taskDefinition

		topoDeps := util.SetFromStrings(taskDefinition.TopologicalDependencies)
		deps := make(util.Set)
		isPackageTask := util.IsPackageTask(taskName)

		for _, dependency := range taskDefinition.TaskDependencies {
			// If the current task is a workspace-specific task (including root Task)
			// and its dependency is _also_ a workspace-specific task, we need to add
			// a reference to this dependency directly into the engine.
			// TODO @mehulkar: Why do we need this?
			if isPackageTask && util.IsPackageTask(dependency) {
				if err := e.AddDep(dependency, taskName); err != nil {
					return err
				}
			} else {
				// For non-workspace-specific dependencies, we attach a reference to
				// the task that is added into the engine.
				deps.Add(dependency)
			}
		}

		// Filter down the tasks if there's a filter in place
		// https: //turbo.build/repo/docs/reference/command-line-reference#--only
		if tasksOnly {
			deps = deps.Filter(func(d interface{}) bool {
				for _, target := range taskNames {
					return fmt.Sprintf("%v", d) == target
				}
				return false
			})
			topoDeps = topoDeps.Filter(func(d interface{}) bool {
				for _, target := range taskNames {
					return fmt.Sprintf("%v", d) == target
				}
				return false
			})
		}

		toTaskID := taskID

		// hasTopoDeps will be true if the task depends on any tasks from dependency packages
		// E.g. `dev: { dependsOn: [^dev] }`
		hasTopoDeps := topoDeps.Len() > 0 && e.completeGraph.WorkspaceGraph.DownEdges(pkg).Len() > 0

		// hasDeps will be true if the task depends on any tasks from its own package
		// E.g. `build: { dependsOn: [dev] }`
		hasDeps := deps.Len() > 0

		// hasPackageTaskDeps will be true if this is a workspace-specific task, and
		// it depends on another workspace-specific tasks
		// E.g. `my-package#build: { dependsOn: [my-package#beforebuild] }`.
		hasPackageTaskDeps := false
		if _, ok := e.PackageTaskDeps[toTaskID]; ok {
			hasPackageTaskDeps = true
		}

		if hasTopoDeps {
			depPkgs := e.completeGraph.WorkspaceGraph.DownEdges(pkg)
			for _, from := range topoDeps.UnsafeListOfStrings() {
				// add task dep from all the package deps within repo
				for depPkg := range depPkgs {
					fromTaskID := util.GetTaskId(depPkg, from)
					e.TaskGraph.Add(fromTaskID)
					e.TaskGraph.Add(toTaskID)
					e.TaskGraph.Connect(dag.BasicEdge(toTaskID, fromTaskID))
					traversalQueue = append(traversalQueue, fromTaskID)
				}
			}
		}

		if hasDeps {
			for _, from := range deps.UnsafeListOfStrings() {
				fromTaskID := util.GetTaskId(pkg, from)
				e.TaskGraph.Add(fromTaskID)
				e.TaskGraph.Add(toTaskID)
				e.TaskGraph.Connect(dag.BasicEdge(toTaskID, fromTaskID))
				traversalQueue = append(traversalQueue, fromTaskID)
			}
		}

		if hasPackageTaskDeps {
			if pkgTaskDeps, ok := e.PackageTaskDeps[toTaskID]; ok {
				for _, fromTaskID := range pkgTaskDeps {
					e.TaskGraph.Add(fromTaskID)
					e.TaskGraph.Add(toTaskID)
					e.TaskGraph.Connect(dag.BasicEdge(toTaskID, fromTaskID))
					traversalQueue = append(traversalQueue, fromTaskID)
				}
			}
		}

		// Add the root node into the graph
		if !hasDeps && !hasTopoDeps && !hasPackageTaskDeps {
			e.TaskGraph.Add(ROOT_NODE_NAME)
			e.TaskGraph.Add(toTaskID)
			e.TaskGraph.Connect(dag.BasicEdge(toTaskID, ROOT_NODE_NAME))
		}
	}

	return nil
}

// AddTask adds root tasks to the engine so they can be looked up later.
func (e *Engine) AddTask(taskName string) {
	if util.IsPackageTask(taskName) {
		pkg, taskName := util.GetPackageTaskFromId(taskName)
		if pkg == util.RootPkgName {
			e.rootEnabledTasks.Add(taskName)
		}
	}
}

// AddDep adds tuples from+to task ID combos in tuple format so they can be looked up later.
func (e *Engine) AddDep(fromTaskID string, toTaskID string) error {
	fromPkg, _ := util.GetPackageTaskFromId(fromTaskID)
	if fromPkg != ROOT_NODE_NAME && fromPkg != util.RootPkgName && !e.completeGraph.WorkspaceGraph.HasVertex(fromPkg) {
		return fmt.Errorf("found reference to unknown package: %v in task %v", fromPkg, fromTaskID)
	}

	if _, ok := e.PackageTaskDeps[toTaskID]; !ok {
		e.PackageTaskDeps[toTaskID] = []string{}
	}

	e.PackageTaskDeps[toTaskID] = append(e.PackageTaskDeps[toTaskID], fromTaskID)

	return nil
}

// ValidatePersistentDependencies checks if any task dependsOn persistent tasks and throws
// an error if that task is actually implemented
func (e *Engine) ValidatePersistentDependencies(graph *graph.CompleteGraph) error {
	var validationError error

	// Adding in a lock because otherwise walking the graph can introduce a data race
	// (reproducible with `go test -race`)
	var sema = util.NewSemaphore(1)

	errs := e.TaskGraph.Walk(func(v dag.Vertex) error {
		vertexName := dag.VertexName(v) // vertexName is a taskID

		// No need to check the root node if that's where we are.
		if strings.Contains(vertexName, ROOT_NODE_NAME) {
			return nil
		}

		// Aquire a lock, because otherwise walking this group can cause a race condition
		// writing to the same validationError var defined outside the Walk(). This shows
		// up when running tests with the `-race` flag.
		sema.Acquire()
		defer sema.Release()

		currentPackageName, currentTaskName := util.GetPackageTaskFromId(vertexName)

		// For each "downEdge" (i.e. each task that _this_ task dependsOn)
		// check if the downEdge is a Persistent task, and if it actually has the script implemented
		// in that package's package.json
		for dep := range e.TaskGraph.DownEdges(vertexName) {
			depTaskID := dep.(string)
			// No need to check the root node
			if strings.Contains(depTaskID, ROOT_NODE_NAME) {
				return nil
			}

			// Parse the taskID of this dependency task
			packageName, taskName := util.GetPackageTaskFromId(depTaskID)

			// Get the Task Definition so we can check if it is Persistent
			depTaskDefinition, taskExists := e.completeGraph.TaskDefinitions[depTaskID]

			if !taskExists {
				return fmt.Errorf("Cannot find task definition for %v in package %v", depTaskID, packageName)
			}

			// Get information about the package
			pkg, pkgExists := graph.WorkspaceInfos[packageName]
			if !pkgExists {
				return fmt.Errorf("Cannot find package %v", packageName)
			}
			_, hasScript := pkg.Scripts[taskName]

			// If both conditions are true set a value and break out of checking the dependencies
			if depTaskDefinition.Persistent && hasScript {
				validationError = fmt.Errorf(
					"\"%s\" is a persistent task, \"%s\" cannot depend on it",
					util.GetTaskId(packageName, taskName),
					util.GetTaskId(currentPackageName, currentTaskName),
				)

				break
			}
		}

		return nil
	})

	for _, err := range errs {
		return fmt.Errorf("Validation failed: %v", err)
	}

	// May or may not be set (could be nil)
	return validationError
}

func (e *Engine) getTaskDefinitionChain(rootPipeline *fs.Pipeline, pkg *fs.PackageJSON, taskID string, taskName string) ([]fs.BookkeepingTaskDefinition, error) {
	// Start a list of TaskDefinitions we've found for this TaskID
	taskDefinitions := []fs.BookkeepingTaskDefinition{}

	// Look for the taskDefinition in the root pipeline.
	// We'll wait to throw errors until the end
	if rootTaskDefinition, err := rootPipeline.GetTask(taskID, taskName); err == nil {
		taskDefinitions = append(taskDefinitions, *rootTaskDefinition)
	}

	// If the taskID is a root task (e.g. //#build), we don't need to look
	// for a workspace task, since these can only be defined in the root turbo.json.
	taskIDPackage, _ := util.GetPackageTaskFromId(taskID)
	if taskIDPackage != util.RootPkgName && taskIDPackage != ROOT_NODE_NAME {
		// Look up task definition in turbo.json in the workspace directory
		workspaceConfigPath := pkg.Dir.Join("turbo.json").RestoreAnchor(e.completeGraph.RepoRoot)

		// If there is an error, we can ignore it, since turbo.json config is not required in the workspace.
		if workspaceTurboJSON, err := fs.ReadTurboConfig(workspaceConfigPath); err == nil {
			// TODO(mehulkar): Enable extending from more than one workspace.
			if len(workspaceTurboJSON.Extends) > 1 {
				return nil, fmt.Errorf(
					"You can only extend from the root workspace. \"%s\" extends from %v",
					pkg.Name,
					workspaceTurboJSON.Extends,
				)
			}

			// TODO(mehulkar):
			// 		Pipeline.GetTask allows searching with a taskID (e.g. `package#task`).
			// 		But we do not want to allow this, except if we're in the root workspace.
			workspaceDefinition, err := workspaceTurboJSON.Pipeline.GetTask(taskID, taskName)

			if err == nil {
				taskDefinitions = append(taskDefinitions, *workspaceDefinition)
			}

			// If there is no Extends key, we are either in a workspace that didn't
			// extend from anything, or in a single package repo, where the workspace is the root.
			if len(workspaceTurboJSON.Extends) == 0 {
				// For the single package case, we can just return the workspaceDefinition
				// (It should be the same as the rootTaskDefinition)
				if pkg.PackageJSONPath == "package.json" {
					return []fs.BookkeepingTaskDefinition{*workspaceDefinition}, nil
				}

				// For turbo.jsons in workspaces that don't have an extends key, throw an error
				// We don't support this right now.
				return nil, fmt.Errorf("No \"extends\" key found in %s", workspaceConfigPath)
			}

			// TODO(mehulkar): Enable extending from non-root workspace.
			if workspaceTurboJSON.Extends[0] != util.RootPkgName {
				return nil, fmt.Errorf(
					"You can only extend from the root workspace. \"%s\" extends from %v",
					pkg.Name,
					workspaceTurboJSON.Extends,
				)
			}
		}
	}

	if len(taskDefinitions) == 0 {
		return nil, fmt.Errorf("Could not find \"%s\" in root turbo.json or \"%s\" workspace", taskID, pkg.Dir)
	}

	return taskDefinitions, nil
}

// GetTaskGraphAncestors gets all the ancestors for a given task in the graph.
// "Ancestors" are all tasks that the given task depends on.
// This is only used by DryRun output right now.
func (e *Engine) GetTaskGraphAncestors(taskID string) ([]string, error) {
	ancestors, err := e.TaskGraph.Ancestors(taskID)
	if err != nil {
		return nil, err
	}
	stringAncestors := []string{}
	for _, dep := range ancestors {
		// Don't leak out internal ROOT_NODE_NAME nodes, which are just placeholders
		if !strings.Contains(dep.(string), ROOT_NODE_NAME) {
			stringAncestors = append(stringAncestors, dep.(string))
		}
	}
	// TODO(mehulkar): Why are ancestors not sorted, but GetTaskGraphDescendants sorts?
	return stringAncestors, nil
}

// GetTaskGraphDescendants gets all the descendants for a given task in the graph.
// "Descendants" are all tasks that depend on the given taskID.
// This is only used by DryRun output right now.
func (e *Engine) GetTaskGraphDescendants(taskID string) ([]string, error) {
	descendents, err := e.TaskGraph.Descendents(taskID)
	if err != nil {
		return nil, err
	}
	stringDescendents := []string{}
	for _, dep := range descendents {
		// Don't leak out internal ROOT_NODE_NAME nodes, which are just placeholders
		if !strings.Contains(dep.(string), ROOT_NODE_NAME) {
			stringDescendents = append(stringDescendents, dep.(string))
		}
	}
	sort.Strings(stringDescendents)
	return stringDescendents, nil
}

func (e *Engine) getPipelineFromWorkspace(workspaceName string) (fs.Pipeline, error) {
	cachedPipeline, ok := e.pipelines[workspaceName]
	if ok {
		return cachedPipeline, nil
	}

	// Note: dir for the root workspace will be an empty string, and for
	// other workspaces, it will be a relative path.
	dir := e.completeGraph.WorkspaceInfos[workspaceName].Dir
	repoRoot := e.completeGraph.RepoRoot
	dirAbsolutePath := dir.RestoreAnchor(repoRoot)

	// We need to a PackageJSON, because LoadTurboConfig requires it as an argument
	// so it can synthesize tasks for single-package repos.
	// In the root workspace, actually get and use the root package.json.
	// For all other workspaces, we don't need the synthesis feature, so we can proceed
	// with a default/blank PackageJSON
	pkgJSON := &fs.PackageJSON{}

	if workspaceName == util.RootPkgName {
		rootPkgJSONPath := dirAbsolutePath.Join("package.json")
		rootPkgJSON, err := fs.ReadPackageJSON(rootPkgJSONPath)
		if err != nil {
			return nil, err
		}
		pkgJSON = rootPkgJSON
	}

	turboConfig, err := fs.LoadTurboConfig(dirAbsolutePath, pkgJSON, e.isSinglePackage)
	if err != nil {
		return nil, err
	}

	// Add to internal cache so we don't have to read file system for every task
	e.pipelines[workspaceName] = turboConfig.Pipeline

	// Return the config from the workspace.
	return e.pipelines[workspaceName], nil
}
