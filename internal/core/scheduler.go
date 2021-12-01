package core

import (
	"fmt"
	"strings"
	"turbo/internal/util"

	"github.com/pyr-sh/dag"
)

const ROOT_NODE_NAME = "___ROOT___"

type Task struct {
	Name string
	// Deps are dependencies between tasks within the same package (e.g. `build` -> `test`)
	Deps util.Set
	// TopoDeps are dependencies across packages within the same topological graph (e.g. parent `build` -> child `build`) */
	TopoDeps util.Set
	Cache    *bool
	Run      func(cwd string) error
}

type scheduler struct {
	// TopologicGraph is a graph of workspaces
	TopologicGraph *dag.AcyclicGraph
	// TaskGraph is a graph of package-tasks
	TaskGraph *dag.AcyclicGraph
	// Tasks are a map of tasks in the scheduler
	Tasks           map[string]*Task
	taskDeps        [][]string
	PackageTaskDeps [][]string
	// Concurrency is the number of concurrent tasks that can be executed
	Concurrency int
	// Parallel is whether to run tasks in parallel
	Parallel bool
}

// NewScheduler creates a new scheduler given a topologic graph of workspace package names
func NewScheduler(topologicalGraph *dag.AcyclicGraph) *scheduler {
	return &scheduler{
		Tasks:           make(map[string]*Task),
		TopologicGraph:  topologicalGraph,
		TaskGraph:       &dag.AcyclicGraph{},
		PackageTaskDeps: [][]string{},
		taskDeps:        [][]string{},
	}
}

// SchedulerExecutionOptions are options for a single scheduler execution
type SchedulerExecutionOptions struct {
	// Packages in the execution scope, if nil, all packages will be considered in scope
	Packages []string
	// TaskNames in the execution scope, if nil, all tasks will be executed
	TaskNames []string
	// Concurrency is the number of concurrent tasks that can be executed
	Concurrency int
	// Parallel is whether to run tasks in parallel
	Parallel bool
}

// Execute executes the pipeline, constructing an internal task graph and walking it accordlingly.
func (p *scheduler) Prepare(options *SchedulerExecutionOptions) error {
	pkgs := options.Packages
	if len(pkgs) == 0 {
		for _, v := range p.TopologicGraph.Vertices() {
			pkgs = append(pkgs, dag.VertexName(v))
		}
	}

	tasks := options.TaskNames
	if len(tasks) == 0 {
		for key := range p.Tasks {
			tasks = append(tasks, key)
		}
	}

	p.Concurrency = options.Concurrency

	p.Parallel = options.Parallel

	if err := p.generateTaskGraph(pkgs, tasks, true); err != nil {
		return err
	}

	return nil
}

func (p *scheduler) Execute() []error {
	var sema = util.NewSemaphore(p.Concurrency)
	return p.TaskGraph.Walk(func(v dag.Vertex) error {
		// Always return if it is the root node
		if strings.Contains(dag.VertexName(v), ROOT_NODE_NAME) {
			return nil
		}
		// Acquire the semaphore unless parallel
		if !p.Parallel {
			sema.Acquire()
			defer sema.Release()
		}
		// Find and run the task for the current vertex
		_, taskName := GetPackageTaskFromId(dag.VertexName(v))
		task, ok := p.Tasks[taskName]
		if !ok {
			return fmt.Errorf("task %s not found", dag.VertexName(v))
		}
		if task.Run != nil {
			return task.Run(dag.VertexName(v))
		}
		return nil
	})
}

func (p *scheduler) generateTaskGraph(scope []string, targets []string, targetsOnly bool) error {
	if p.PackageTaskDeps == nil {
		p.PackageTaskDeps = [][]string{}
	}

	packageTasksDepsMap := getPackageTaskDepsMap(p.PackageTaskDeps)

	taskDeps := [][]string{}

	traversalQueue := []string{}

	for _, pkg := range scope {
		for _, target := range targets {
			traversalQueue = append(traversalQueue, GetTaskId(pkg, target))
		}
	}

	visited := make(util.Set)

	for len(traversalQueue) > 0 {
		taskId := traversalQueue[0]
		traversalQueue = traversalQueue[1:]
		pkg, taskName := GetPackageTaskFromId(taskId)
		task, ok := p.Tasks[taskName]
		if !ok {
			return fmt.Errorf("task %v not found", taskId)
		}
		if !visited.Include(taskId) {
			visited.Add(taskId)
			deps := task.Deps

			if targetsOnly {
				deps = deps.Filter(func(d interface{}) bool {
					for _, target := range targets {
						if dag.VertexName(d) == target {
							return true
						}
					}
					return false
				})
			}

			toTaskId := GetTaskId(pkg, taskName)
			hasTopoDeps := task.TopoDeps.Len() > 0 && p.TopologicGraph.DownEdges(pkg).Len() > 0
			hasDeps := deps.Len() > 0
			hasPackageTaskDeps := false
			if _, ok := packageTasksDepsMap[toTaskId]; ok {
				hasPackageTaskDeps = true
			}

			if hasTopoDeps {
				for _, from := range task.TopoDeps.UnsafeListOfStrings() {
					// TODO: this should move out of the loop???
					depPkgs, err := p.TopologicGraph.Ancestors(pkg)
					if err != nil {
						return fmt.Errorf("error getting ancestors: %w", err)
					}

					// add task dep from all the package deps within repo
					for _, depPkg := range depPkgs.List() {
						fromTaskId := GetTaskId(depPkg, from)
						taskDeps = append(taskDeps, []string{fromTaskId, toTaskId})
						p.TaskGraph.Add(fromTaskId)
						p.TaskGraph.Add(toTaskId)
						p.TaskGraph.Connect(dag.BasicEdge(toTaskId, fromTaskId))
						traversalQueue = append(traversalQueue, fromTaskId)
					}
				}
			}

			if hasDeps {
				for _, from := range deps.UnsafeListOfStrings() {
					fromTaskId := GetTaskId(pkg, from)
					taskDeps = append(taskDeps, []string{fromTaskId, toTaskId})
					p.TaskGraph.Add(fromTaskId)
					p.TaskGraph.Add(toTaskId)
					p.TaskGraph.Connect(dag.BasicEdge(toTaskId, fromTaskId))
					traversalQueue = append(traversalQueue, fromTaskId)
				}
			}

			if hasPackageTaskDeps {
				if pkgTaskDeps, ok := packageTasksDepsMap[toTaskId]; ok {
					for _, fromTaskId := range pkgTaskDeps {
						taskDeps = append(taskDeps, []string{fromTaskId, toTaskId})
						p.TaskGraph.Add(fromTaskId)
						p.TaskGraph.Add(toTaskId)
						p.TaskGraph.Connect(dag.BasicEdge(toTaskId, fromTaskId))
						traversalQueue = append(traversalQueue, fromTaskId)
					}
				}
			}

			if !hasDeps && !hasTopoDeps && !hasPackageTaskDeps {
				// TODO: this should change to ROOT_NODE_NAME
				fromTaskId := GetTaskId(pkg, "")
				taskDeps = append(taskDeps, []string{fromTaskId, toTaskId})
				p.TaskGraph.Add(ROOT_NODE_NAME)
				p.TaskGraph.Add(toTaskId)
				p.TaskGraph.Connect(dag.BasicEdge(toTaskId, ROOT_NODE_NAME))
			}
		}
	}
	p.taskDeps = taskDeps
	return nil
}

func getPackageTaskDepsMap(packageTaskDeps [][]string) map[string][]string {
	depMap := make(map[string][]string)
	for _, packageTaskDep := range packageTaskDeps {
		from := packageTaskDep[0]
		to := packageTaskDep[1]
		if _, ok := depMap[to]; !ok {
			depMap[to] = []string{}
		}
		depMap[to] = append(depMap[to], from)
	}
	return depMap
}

func (p *scheduler) AddTask(task *Task) *scheduler {
	p.Tasks[task.Name] = task
	return p
}

func (p *scheduler) AddDep(fromTaskId string, toTaskId string) *scheduler {
	p.PackageTaskDeps = append(p.PackageTaskDeps, []string{fromTaskId, toTaskId})
	return p
}
