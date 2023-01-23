package core

import (
	"fmt"
	"strings"

	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/graph"
	"github.com/vercel/turbo/cli/internal/turbopath"
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
	// TopologicGraph is a graph of workspaces
	TopologicGraph *dag.AcyclicGraph
	// TaskGraph is a graph of package-tasks
	TaskGraph *dag.AcyclicGraph
	// Tasks are a map of tasks in the engine
	Tasks            map[string]*Task
	PackageTaskDeps  map[string][]string
	rootEnabledTasks util.Set

	// completeGraph is the CompleteGraph. We need this to look up the Pipeline, etc.
	completeGraph *graph.CompleteGraph
}

// NewEngine creates a new engine given a topologic graph of workspace package names
func NewEngine(completeGraph *graph.CompleteGraph) *Engine {
	return &Engine{
		completeGraph:    completeGraph,
		Tasks:            make(map[string]*Task),
		TopologicGraph:   &completeGraph.WorkspaceGraph,
		TaskGraph:        &dag.AcyclicGraph{},
		PackageTaskDeps:  map[string][]string{},
		rootEnabledTasks: make(util.Set),
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

// Prepare constructs the Task Graph for a list of packages and tasks
func (e *Engine) Prepare(options *EngineBuildingOptions) error {
	pkgs := options.Packages
	tasks := options.TaskNames
	if len(tasks) == 0 {
		// TODO(gsoltis): Is this behavior used?
		for key := range e.Tasks {
			tasks = append(tasks, key)
		}
	}

	if err := e.generateTaskGraph(pkgs, tasks, options.TasksOnly); err != nil {
		return err
	}

	return nil
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

func (e *Engine) getTaskDefinition(taskName string, taskID string) (*Task, error) {
	if task, ok := e.Tasks[taskID]; ok {
		return task, nil
	}
	if task, ok := e.Tasks[taskName]; ok {
		return task, nil
	}

	return nil, fmt.Errorf("Missing task definition, configure \"%s\" or \"%s\" in turbo.json", taskName, taskID)
}

func (e *Engine) generateTaskGraph(pkgs []string, taskNames []string, tasksOnly bool) error {
	traversalQueue := []string{}

	for _, pkg := range pkgs {
		isRootPkg := pkg == util.RootPkgName

		for _, taskName := range taskNames {
			if !isRootPkg || e.rootEnabledTasks.Includes(taskName) {
				taskID := util.GetTaskId(pkg, taskName)
				if _, err := e.getTaskDefinition(taskName, taskID); err != nil {
					// Initial, non-package tasks are not required to exist, as long as some
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

		// Skip this iteration of the loop if we've already seen this taskID
		if visited.Includes(taskID) {
			continue
		}

		visited.Add(taskID)

		pkg, ok := e.completeGraph.WorkspaceInfos[pkg]

		if !ok {
			// This should be unlikely to happen. If we have a pkg
			// it should be in WorkspaceInfos. If we're hitting this error
			// something has gone wrong earlier when building WorkspaceInfos
			return fmt.Errorf("Failed to look up workspace %s", pkg)
		}

		fmt.Printf("[debug] e.completeGraph.Pipeline %#v\n", e.completeGraph.Pipeline)

		taskDefinition, err := e.GetResolvedTaskDefinition(
			&e.completeGraph.Pipeline,
			pkg,
			taskID,
			taskName,
		)

		if err != nil {
			return err
		}

		// Put this taskDefinition into the Graph so we can look it up later during execution.
		e.completeGraph.TaskDefinitions[taskID] = taskDefinition

		topoDeps := util.SetFromStrings(taskDefinition.TopologicalDependencies)
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
		hasTopoDeps := topoDeps.Len() > 0 && e.TopologicGraph.DownEdges(pkg).Len() > 0

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
			depPkgs := e.TopologicGraph.DownEdges(pkg)
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

// AddTask adds a task to the Engine so it can be looked up later.
func (e *Engine) AddTask(task *Task) *Engine {
	// If a root task is added, mark the task name as eligible for
	// root execution. Otherwise, it will be skipped.
	if util.IsPackageTask(task.Name) {
		pkg, taskName := util.GetPackageTaskFromId(task.Name)
		if pkg == util.RootPkgName {
			e.rootEnabledTasks.Add(taskName)
		}
	}

	// TODO(mehulkar): Now that we're composing taskDefinition
	// do we even need to store these in the engine? Should we instead store the resolved
	// TaskDefinition here?
	e.Tasks[task.Name] = task
	return e
}

// AddDep adds tuples from+to task ID combos in tuple format so they can be looked up later.
func (e *Engine) AddDep(fromTaskID string, toTaskID string) error {
	fromPkg, _ := util.GetPackageTaskFromId(fromTaskID)
	if fromPkg != ROOT_NODE_NAME && fromPkg != util.RootPkgName && !e.TopologicGraph.HasVertex(fromPkg) {
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
			pkg, taskName := util.GetPackageTaskFromId(depTaskID)

			// Get the Task Definition so we can check if it is Persistent
			// TODO(mehulkar): Do we need to get a resolved taskDefinition here?
			depTaskDefinition, taskExists := e.getTaskDefinition(taskName, depTaskID)
			if taskExists != nil {
				return fmt.Errorf("Cannot find task definition for %v in package %v", depTaskID, pkg)
			}

			// Get information about the package
			pkg, pkgExists := graph.WorkspaceInfos[pkg]
			if !pkgExists {
				return fmt.Errorf("Cannot find package %v", pkg)
			}
			_, hasScript := pkg.Scripts[taskName]

			// If both conditions are true set a value and break out of checking the dependencies
			if depTaskDefinition.TaskDefinition.Persistent && hasScript {
				validationError = fmt.Errorf(
					"\"%s\" is a persistent task, \"%s\" cannot depend on it",
					util.GetTaskId(pkg, taskName),
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

// GetResolvedTaskDefinition returns a "resolved" TaskDefinition composed of one
// turbo.json in the workspace and following any `extends` keys up. If there is
// no turbo.json in the workspace, returns the taskDefinition from the root Pipeline.
func (e *Engine) GetResolvedTaskDefinition(rootPipeline *fs.Pipeline, pkg *fs.PackageJSON, taskID string, taskName string) (*fs.TaskDefinition, error) {
	taskDefinitions, err := e.getTaskDefinitionChain(rootPipeline, pkg, taskID, taskName)
	if err != nil {
		return nil, err
	}

	// reverse the array, because we want to start with the end of the chain.
	for i, j := 0, len(taskDefinitions)-1; i < j; i, j = i+1, j-1 {
		taskDefinitions[i], taskDefinitions[j] = taskDefinitions[j], taskDefinitions[i]
	}

	// Start with an empty definition
	mergedTaskDefinition := &fs.TaskDefinition{}

	// For each of the TaskDefinitions we know of, merge them in
	for _, taskDef := range taskDefinitions {
		mergedTaskDefinition.Outputs = taskDef.Outputs
		mergedTaskDefinition.ShouldCache = taskDef.ShouldCache
		mergedTaskDefinition.EnvVarDependencies = taskDef.EnvVarDependencies
		mergedTaskDefinition.TopologicalDependencies = taskDef.TopologicalDependencies
		mergedTaskDefinition.TaskDependencies = taskDef.TaskDependencies
		mergedTaskDefinition.Inputs = taskDef.Inputs
		mergedTaskDefinition.OutputMode = taskDef.OutputMode
		mergedTaskDefinition.Persistent = taskDef.Persistent
	}

	return mergedTaskDefinition, nil
}

func (e *Engine) getTaskDefinitionChain(rootPipeline *fs.Pipeline, pkg *fs.PackageJSON, taskID string, taskName string) ([]fs.TaskDefinition, error) {
	// Start a list of TaskDefinitions we've found for this TaskID
	taskDefinitions := []fs.TaskDefinition{}

	// Start in the workspace directory
	turboJSONPath := turbopath.AbsoluteSystemPath(pkg.Dir).UntypedJoin("turbo.json")
	_, err := fs.ReadTurboConfig(turboJSONPath)

	// If there is no turbo.json in the workspace directory, we'll use the one in root turbo.json
	if err != nil {
		fmt.Printf("[debug] root: rootPipeline %#v\n", rootPipeline)
		// fmt.Printf("[debug] root: Looking up def for %#v, %#v\n", taskID, taskName)
		rootTaskDefinition, err := rootPipeline.GetTask(taskID, taskName)
		if err != nil {
			// This should be an unlikely error scenario. If we're working with a task
			// there should be a definition in the rootPipeline. So an error here suggests
			// that something else went wrong before we got here.
			return nil, err
		}
		taskDefinitions = append(taskDefinitions, *rootTaskDefinition)
		return taskDefinitions, nil
	}

	graph := e.completeGraph

	// For loop until we `break` manually.
	// We will reassign `turboJSONPath` inside this loop, so that
	// every time we iterate, we're starting from a new one.
	for {
		turboJSON, err := fs.ReadTurboConfig(turboJSONPath)
		if err != nil {
			return nil, err
		}

		// TODO(mehulkar):
		// 		getTaskFromPipeline allows searching with a taskID (e.g. `package#task`).
		// 		But we do not want to allow this, except if we're in the root workspace.
		fmt.Printf("[debug] Looking up def for %#v, %#v\n", taskID, taskName)
		taskDefinition, err := turboJSON.Pipeline.GetTask(taskID, taskName)
		if err != nil {
			// If there was nothing in the pipeline for this task
			// We can exit
			break
		} else {
			// Add it into the taskDefinitions
			taskDefinitions = append(taskDefinitions, *taskDefinition)

			// If this turboJSON doesn't have an extends property, we can stop our for loop here.
			if len(turboJSON.Extends) == 0 {
				break
			}

			// TODO(mehulkar): Enable extending from more than one workspace.
			// TODO(mehulkar): Enable extending from non-root workspace.
			if len(turboJSON.Extends) > 1 || turboJSON.Extends[0] != util.RootPkgName {
				// TODO(mehulkar): Using pkg.Name here is wrong, since pkg changes on each iteration
				return nil, fmt.Errorf(
					"You can only extend from the root workspace. \"%s\" extends from %v",
					pkg.Name,
					turboJSON.Extends,
				)
			}

			// If there's an extends property, walk up to the next one, find the workspace it refers to,
			// and and assign `directory` to it for the next iteration in this for loop.
			// Note(mehulkar):
			//		We are looping through all items in Extends, but as of now,
			// 		and based on the checks above, we only want to read the first item
			// 		(and we already know that it's the root workspace).
			for _, workspaceName := range turboJSON.Extends {
				workspace, ok := graph.WorkspaceInfos[workspaceName]
				if !ok {
					// TODO: Should this be a hard error?
					// A workspace was referenced that doesn't exist or we know nothing about
					break
				}

				// Reassign these. The loop will run again with this new turbo.json now.
				turboJSONPath = turbopath.AbsoluteSystemPath(workspace.Dir).UntypedJoin("turbo.json")
			}
		}
	}

	return taskDefinitions, nil
}
