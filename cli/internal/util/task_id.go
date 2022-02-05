package util

import (
	"fmt"
	"strings"
)

const TASK_DELIMITER = "#"

// GetTaskId returns a package-task identifier (e.g @feed/thing#build).
func GetTaskId(pkgName interface{}, target string) string {
	if IsPackageTask(target) {
		return target
	}
	return fmt.Sprintf("%v%v%v", pkgName, TASK_DELIMITER, target)
}

// GetPackageTaskFromId returns a tuple of the package name and target task
func GetPackageTaskFromId(taskId string) (packageName string, task string) {
	arr := strings.Split(taskId, TASK_DELIMITER)
	return arr[0], arr[1]
}

// IsPackageTask returns true if a is a package-specific task (e.g. myapp#build)
func IsPackageTask(task string) bool {
	return strings.Contains(task, TASK_DELIMITER)
}
