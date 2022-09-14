package util

import (
	"fmt"
	"strings"
)

const (
	// TaskDelimiter separates a package name from a task name in a task id
	TaskDelimiter = "#"
	// RootPkgName is the reserved name that specifies the root package
	RootPkgName = "//"
)

// GetTaskId returns a package-task identifier (e.g @feed/thing#build).
func GetTaskId(pkgName interface{}, target string) string {
	if IsPackageTask(target) {
		return target
	}
	return fmt.Sprintf("%v%v%v", pkgName, TaskDelimiter, target)
}

// RootTaskID returns the task id for running the given task in the root package
func RootTaskID(target string) string {
	return GetTaskId(RootPkgName, target)
}

// GetPackageTaskFromId returns a tuple of the package name and target task
func GetPackageTaskFromId(taskId string) (packageName string, task string) {
	arr := strings.Split(taskId, TaskDelimiter)
	return arr[0], arr[1]
}

func RootTaskTaskName(taskID string) string {
	return strings.TrimPrefix(taskID, RootPkgName+TaskDelimiter)
}

// IsPackageTask returns true if a is a package-specific task (e.g. myapp#build)
func IsPackageTask(task string) bool {
	return strings.Contains(task, TaskDelimiter)
}
