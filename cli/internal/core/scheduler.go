package core

import (
	"fmt"
	"strings"

	"github.com/vercel/turborepo/cli/internal/util"

	"github.com/pyr-sh/dag"
)

const ROOT_NODE_NAME = "___ROOT___"

type Task struct {
	Name string
	// Deps are dependencies between tasks within the same package (e.g. `build` -> `test`)
	Deps util.Set
	// TopoDeps are dependencies across packages within the same topological graph (e.g. parent `build` -> child `build`) */
	TopoDeps util.Set
}

type Visitor = func(taskID string) error

type Scheduler struct {
	// TopologicGraph is a graph of workspaces
	TopologicGraph *dag.AcyclicGraph
	// TaskGraph is a graph of package-tasks
	TaskGraph *dag.AcyclicGraph
	// Tasks are a map of tasks in the scheduler
	Tasks           map[string]*Task
	taskDeps        [][]string
	PackageTaskDeps [][]string
}

// NewScheduler creates a new scheduler given a topologic graph of workspace package names
func NewScheduler(topologicalGraph *dag.AcyclicGraph) *Scheduler {
	return &Scheduler{
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
	// Restrict execution to only the listed task names
	TasksOnly bool
}

func (p *Scheduler) Prepare(options *SchedulerExecutionOptions) error {
	pkgs := options.Packages
	if len(pkgs) == 0 {
		// TODO(gsoltis): Is this behavior only used in tests?
		for _, v := range p.TopologicGraph.Vertices() {
			pkgs = append(pkgs, dag.VertexName(v))
		}
	}

	tasks := options.TaskNames
	if len(tasks) == 0 {
		// TODO(gsoltis): Is this behavior used?
		for key := range p.Tasks {
			tasks = append(tasks, key)
		}
	}

	if err := p.generateTaskGraph(pkgs, tasks, options.TasksOnly); err != nil {
		return err
	}

	return nil
}

// ExecOpts controls a single walk of the task graph
type ExecOpts struct {
	// Parallel is whether to run tasks in parallel
	Parallel bool
	// Concurrency is the number of concurrent tasks that can be executed
	Concurrency int
}

// Execute executes the pipeline, constructing an internal task graph and walking it accordingly.
func (p *Scheduler) Execute(visitor Visitor, opts ExecOpts) []error {
	var sema = util.NewSemaphore(opts.Concurrency)
	return p.TaskGraph.Walk(func(v dag.Vertex) error {
		// Always return if it is the root node
		if strings.Contains(dag.VertexName(v), ROOT_NODE_NAME) {
			return nil
		}
		// Acquire the semaphore unless parallel
		if !opts.Parallel {
			sema.Acquire()
			defer sema.Release()
		}
		return visitor(dag.VertexName(v))
	})
}

func (p *Scheduler) generateTaskGraph(scope []string, taskNames []string, tasksOnly bool) error {
	if p.PackageTaskDeps == nil {
		p.PackageTaskDeps = [][]string{}
	}

	packageTasksDepsMap := getPackageTaskDepsMap(p.PackageTaskDeps)

	taskDeps := [][]string{}

	traversalQueue := []string{}

	for _, pkg := range scope {
		for _, target := range taskNames {
			traversalQueue = append(traversalQueue, util.GetTaskId(pkg, target))
		}
	}

	visited := make(util.Set)

	for len(traversalQueue) > 0 {
		taskId := traversalQueue[0]
		traversalQueue = traversalQueue[1:]
		pkg, taskName := util.GetPackageTaskFromId(taskId)
		task, ok := p.Tasks[taskName]
		if !ok {
			return fmt.Errorf("task %v not found", taskId)
		}
		if !visited.Includes(taskId) {
			visited.Add(taskId)
			deps := task.Deps

			if tasksOnly {
				deps = deps.Filter(func(d interface{}) bool {
					for _, target := range taskNames {
						return fmt.Sprintf("%v", d) == target
					}
					return false
				})
				task.TopoDeps = task.TopoDeps.Filter(func(d interface{}) bool {
					for _, target := range taskNames {
						return fmt.Sprintf("%v", d) == target
					}
					return false
				})
			}

			toTaskId := util.GetTaskId(pkg, taskName)
			hasTopoDeps := task.TopoDeps.Len() > 0 && p.TopologicGraph.DownEdges(pkg).Len() > 0
			hasDeps := deps.Len() > 0
			hasPackageTaskDeps := false
			if _, ok := packageTasksDepsMap[toTaskId]; ok {
				hasPackageTaskDeps = true
			}

			if hasTopoDeps {
        depPkgs := p.TopologicGraph.DownEdges(pkg)
				for _, from := range task.TopoDeps.UnsafeListOfStrings() {
					// add task dep from all the package deps within repo
					for depPkg := range depPkgs {
						fromTaskId := util.GetTaskId(depPkg, from)
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
					fromTaskId := util.GetTaskId(pkg, from)
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
				fromTaskId := util.GetTaskId(pkg, "")
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

func (p *Scheduler) AddTask(task *Task) *Scheduler {
	p.Tasks[task.Name] = task
	return p
}

func (p *Scheduler) AddDep(fromTaskId string, toTaskId string) *Scheduler {
	p.PackageTaskDeps = append(p.PackageTaskDeps, []string{fromTaskId, toTaskId})
	return p
}
