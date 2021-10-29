package core

import (
	"fmt"
	"turbo/internal/util"

	"github.com/pyr-sh/dag"
)

type Task struct {
	Name     string
	Deps     util.Set
	TopoDeps util.Set
	Run      func(cwd string) error
}

type pipeline struct {
	TopologicGraph  *dag.AcyclicGraph
	TaskGraph       *dag.AcyclicGraph
	Tasks           map[string]*Task
	taskDeps        [][]string
	PackageTaskDeps [][]string
}

func New(topologicalGraph *dag.AcyclicGraph) *pipeline {
	return &pipeline{
		Tasks:          make(map[string]*Task),
		TopologicGraph: topologicalGraph,
		TaskGraph:      &dag.AcyclicGraph{},
		taskDeps:       [][]string{},
	}
}

func (p *pipeline) Run(packages []string, taskNames []string) []error {
	pkgs := packages
	if len(pkgs) == 0 {
		for _, v := range p.TopologicGraph.Vertices() {
			pkgs = append(pkgs, dag.VertexName(v))
		}
	}

	tasks := taskNames
	if len(tasks) == 0 {
		for key := range p.Tasks {
			tasks = append(tasks, key)
		}
	}

	if err := p.generateTaskGraph(pkgs, tasks, true); err != nil {
		return []error{err}
	}

	return p.TaskGraph.Walk(func(v dag.Vertex) error {
		if dag.VertexName(v) == "root" {
			return nil
		}
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

func (p *pipeline) generateTaskGraph(scope []string, targets []string, targetsOnly bool) error {
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
				p.TaskGraph.Add("root")
				p.TaskGraph.Add(toTaskId)
				p.TaskGraph.Connect(dag.BasicEdge(toTaskId, "root"))
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

func (p *pipeline) AddTask(task *Task) *pipeline {
	p.Tasks[task.Name] = task
	return p
}

func (p *pipeline) AddDep(task *Task) *pipeline {
	p.Tasks[task.Name] = task
	return p
}
