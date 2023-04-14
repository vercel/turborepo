package core

import (
	"errors"
	"fmt"
	"os"
	"sort"
	"strings"
	"sync/atomic"

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
	var errored int32
	return e.TaskGraph.Walk(func(v dag.Vertex) error {
		// If something has already errored, short-circuit.
		// There is a race here between concurrent tasks. However, if there is not a
		// dependency edge between them, we are not required to have a strict order
		// between them, so a failed task can fail to short-circuit a concurrent
		// task that happened to be starting at the same time.
		if atomic.LoadInt32(&errored) != 0 {
			return nil
		}
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

		if err := visitor(taskID); err != nil {
			// We only ever flip from false to true, so we don't need to compare and swap the atomic
			atomic.StoreInt32(&errored, 1)
			return err
		}
		return nil
	})
}

// MissingTaskError is a specialized Error thrown in the case that we can't find a task.
// We want to allow this error when getting task definitions, so we have to special case it.
type MissingTaskError struct {
	workspaceName string
	taskID        string
	taskName      string
}

func (m *MissingTaskError) Error() string {
	return fmt.Sprintf("Could not find \"%s\" or \"%s\" in workspace \"%s\"", m.taskName, m.taskID, m.workspaceName)
}

func (e *Engine) getTaskDefinition(pkg string, taskName string, taskID string) (*Task, error) {
	pipeline, err := e.completeGraph.GetPipelineFromWorkspace(pkg, e.isSinglePackage)

	if err != nil {
		if pkg != util.RootPkgName {
			// If there was no turbo.json in the workspace, fallback to the root turbo.json
			if errors.Is(err, os.ErrNotExist) {
				return e.getTaskDefinition(util.RootPkgName, taskName, taskID)
			}

			// otherwise bubble it up
			return nil, err
		}

		return nil, err
	}

	if task, ok := pipeline[taskID]; ok {
		return &Task{
			Name:           taskName,
			TaskDefinition: task.GetTaskDefinition(),
		}, nil
	}

	if task, ok := pipeline[taskName]; ok {
		return &Task{
			Name:           taskName,
			TaskDefinition: task.GetTaskDefinition(),
		}, nil
	}

	// An error here means turbo.json exists, but didn't define the task.
	// Fallback to the root pipeline to find the task.
	if pkg != util.RootPkgName {
		return e.getTaskDefinition(util.RootPkgName, taskName, taskID)
	}

	// Return this as a custom type so we can ignore it specifically
	return nil, &MissingTaskError{
		taskName:      taskName,
		taskID:        taskID,
		workspaceName: pkg,
	}
}

// Prepare constructs the Task Graph for a list of packages and tasks
func (e *Engine) Prepare(options *EngineBuildingOptions) error {
	pkgs := options.Packages
	taskNames := options.TaskNames
	tasksOnly := options.TasksOnly

	// If there are no affected packages, we don't need to go through all this work
	// we can just exit early.
	// TODO(mehulkar): but we still need to validate bad task names?
	if len(pkgs) == 0 {
		return nil
	}

	traversalQueue := []string{}

	// get a set of taskNames passed in. we'll remove the ones that have a definition
	missing := util.SetFromStrings(taskNames)

	// Get a list of entry points into our TaskGraph.
	// We do this by taking the input taskNames, and pkgs
	// and creating a queue of taskIDs that we can traverse and gather dependencies from.
	for _, pkg := range pkgs {
		for _, taskName := range taskNames {
			taskID := util.GetTaskId(pkg, taskName)

			// Look up the task in the package
			foundTask, err := e.getTaskDefinition(pkg, taskName, taskID)

			// We can skip MissingTaskErrors because we'll validate against them later
			// Return all other errors
			if err != nil {
				var e *MissingTaskError
				if errors.As(err, &e) {
					// Initially, non-package tasks are not required to exist, as long as some
					// package in the list packages defines it as a package-task. Dependencies
					// *are* required to have a definition.
					continue
				}

				return err
			}

			// If we found a task definition, remove it from the missing list
			if foundTask != nil {
				// delete taskName if it was found
				missing.Delete(taskName)

				// Even if a task definition was found, we _only_ want to add it as an entry point to
				// the task graph (i.e. the traversalQueue), if it's:
				// - A task from the non-root workspace (i.e. tasks from every other workspace)
				// - A task that we *know* is rootEnabled task (in which case, the root workspace is acceptable)
				isRootPkg := pkg == util.RootPkgName
				if !isRootPkg || e.rootEnabledTasks.Includes(taskName) {
					traversalQueue = append(traversalQueue, taskID)
				}
			}
		}
	}

	visited := make(util.Set)

	// validate that all tasks passed were found
	missingList := missing.UnsafeListOfStrings()
	sort.Strings(missingList)

	if len(missingList) > 0 {
		return fmt.Errorf("Could not find the following tasks in project: %s", strings.Join(missingList, ", "))
	}

	// Things get appended to traversalQueue inside this loop, so we use the len() check instead of range.
	for len(traversalQueue) > 0 {
		// pop off the first item from the traversalQueue
		taskID := traversalQueue[0]
		traversalQueue = traversalQueue[1:]

		pkg, taskName := util.GetPackageTaskFromId(taskID)

		if pkg == util.RootPkgName && !e.rootEnabledTasks.Includes(taskName) {
			return fmt.Errorf("%v needs an entry in turbo.json before it can be depended on because it is a task run from the root package", taskID)
		}

		if pkg != ROOT_NODE_NAME {
			if _, ok := e.completeGraph.WorkspaceInfos.PackageJSONs[pkg]; !ok {
				// If we have a pkg it should be in WorkspaceInfos.
				// If we're hitting this error something has gone wrong earlier when building WorkspaceInfos
				// or the workspace really doesn't exist and turbo.json is misconfigured.
				return fmt.Errorf("Could not find workspace \"%s\" from task \"%s\" in project", pkg, taskID)
			}
		}

		taskDefinitions, err := e.getTaskDefinitionChain(taskID, taskName)
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
func (e *Engine) ValidatePersistentDependencies(graph *graph.CompleteGraph, concurrency int) error {
	var validationError error
	persistentCount := 0

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

		currentTaskDefinition, currentTaskExists := e.completeGraph.TaskDefinitions[vertexName]
		if currentTaskExists && currentTaskDefinition.Persistent {
			persistentCount++
		}

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
			pkg, pkgExists := graph.WorkspaceInfos.PackageJSONs[packageName]
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

	if validationError != nil {
		return validationError
	} else if persistentCount >= concurrency {
		return fmt.Errorf("You have %v persistent tasks but `turbo` is configured for concurrency of %v. Set --concurrency to at least %v", persistentCount, concurrency, persistentCount+1)
	}

	return nil
}

// getTaskDefinitionChain gets a set of TaskDefinitions that apply to the taskID.
// These definitions should be merged by the consumer.
func (e *Engine) getTaskDefinitionChain(taskID string, taskName string) ([]fs.BookkeepingTaskDefinition, error) {
	// Start a list of TaskDefinitions we've found for this TaskID
	taskDefinitions := []fs.BookkeepingTaskDefinition{}

	rootPipeline, err := e.completeGraph.GetPipelineFromWorkspace(util.RootPkgName, e.isSinglePackage)
	if err != nil {
		// It should be very unlikely that we can't find a root pipeline. Even for single package repos
		// the pipeline is synthesized from package.json, so there should be _something_ here.
		return nil, err
	}

	// Look for the taskDefinition in the root pipeline.
	if rootTaskDefinition, err := rootPipeline.GetTask(taskID, taskName); err == nil {
		taskDefinitions = append(taskDefinitions, *rootTaskDefinition)
	}

	// If we're in a single package repo, we can just exit with the TaskDefinition in the root pipeline
	// since there are no workspaces, and we don't need to follow any extends keys.
	if e.isSinglePackage {
		if len(taskDefinitions) == 0 {
			return nil, fmt.Errorf("Could not find \"%s\" in root turbo.json", taskID)
		}
		return taskDefinitions, nil
	}

	// If the taskID is a root task (e.g. //#build), we don't need to look
	// for a workspace task, since these can only be defined in the root turbo.json.
	taskIDPackage, _ := util.GetPackageTaskFromId(taskID)
	if taskIDPackage != util.RootPkgName && taskIDPackage != ROOT_NODE_NAME {
		// If there is an error, we can ignore it, since turbo.json config is not required in the workspace.
		if workspaceTurboJSON, err := e.completeGraph.GetTurboConfigFromWorkspace(taskIDPackage, e.isSinglePackage); err != nil {
			// swallow the error where the config file doesn't exist, but bubble up other things
			if !errors.Is(err, os.ErrNotExist) {
				return nil, err
			}
		} else {
			// Run some validations on a workspace turbo.json. Note that these validations are on
			// the whole struct, and not relevant to the taskID we're looking at right now.
			validationErrors := workspaceTurboJSON.Validate([]fs.TurboJSONValidation{
				validateNoPackageTaskSyntax,
				validateExtends,
			})

			if len(validationErrors) > 0 {
				fullError := errors.New("Invalid turbo.json")
				for _, validationErr := range validationErrors {
					fullError = fmt.Errorf("%w\n - %s", fullError, validationErr)
				}

				return nil, fullError
			}

			// If there are no errors, we can (try to) add the TaskDefinition to our list.
			if workspaceDefinition, ok := workspaceTurboJSON.Pipeline[taskName]; ok {
				taskDefinitions = append(taskDefinitions, workspaceDefinition)
			}
		}
	}

	if len(taskDefinitions) == 0 {
		return nil, fmt.Errorf("Could not find \"%s\" in root turbo.json or \"%s\" workspace", taskID, taskIDPackage)
	}

	return taskDefinitions, nil
}

func validateNoPackageTaskSyntax(turboJSON *fs.TurboJSON) []error {
	errors := []error{}

	for taskIDOrName := range turboJSON.Pipeline {
		if util.IsPackageTask(taskIDOrName) {
			taskName := util.StripPackageName(taskIDOrName)
			errors = append(errors, fmt.Errorf("\"%s\". Use \"%s\" instead", taskIDOrName, taskName))
		}
	}

	return errors
}

func validateExtends(turboJSON *fs.TurboJSON) []error {
	extendErrors := []error{}
	extends := turboJSON.Extends
	// TODO(mehulkar): Enable extending from more than one workspace.
	if len(extends) > 1 {
		extendErrors = append(extendErrors, fmt.Errorf("You can only extend from the root workspace"))
	}

	// We don't support this right now
	if len(extends) == 0 {
		extendErrors = append(extendErrors, fmt.Errorf("No \"extends\" key found"))
	}

	// TODO(mehulkar): Enable extending from non-root workspace.
	if len(extends) == 1 && extends[0] != util.RootPkgName {
		extendErrors = append(extendErrors, fmt.Errorf("You can only extend from the root workspace"))
	}

	return extendErrors
}
