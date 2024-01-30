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

// RootTaskTaskName returns the task portion of a root task taskID
func RootTaskTaskName(taskID string) string {
	return strings.TrimPrefix(taskID, RootPkgName+TaskDelimiter)
}

// IsPackageTask returns true if input is a package-specific task
// whose name has a length greater than 0.
//
// Accepted: myapp#build
// Rejected: #build, build
func IsPackageTask(task string) bool {
	return strings.Index(task, TaskDelimiter) > 0
}

// IsTaskInPackage returns true if the task does not belong to a different package
// note that this means unscoped tasks will always return true
func IsTaskInPackage(task string, packageName string) bool {
	if !IsPackageTask(task) {
		return true
	}
	packageNameExpected, _ := GetPackageTaskFromId(task)
	return packageNameExpected == packageName
}

// StripPackageName removes the package portion of a taskID if it
// is a package task. Non-package tasks are returned unmodified
func StripPackageName(taskID string) string {
	if IsPackageTask(taskID) {
		_, task := GetPackageTaskFromId(taskID)
		return task
	}
	return taskID
}
