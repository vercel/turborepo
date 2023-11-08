// Package nodes defines the nodes that are present in the execution graph used by turbo.
package nodes

import (
	"fmt"
	"path/filepath"
	"strings"

	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/fs/hash"
	"github.com/vercel/turbo/cli/internal/util"
)

// PackageTask represents running a particular task in a particular package
type PackageTask struct {
	TaskID          string
	Task            string
	PackageName     string
	Pkg             *fs.PackageJSON
	EnvMode         util.EnvMode
	TaskDefinition  *fs.TaskDefinition
	Dir             string
	Command         string
	Outputs         []string
	ExcludedOutputs []string
	Hash            string
}

const logDir = ".turbo"

// RepoRelativeSystemLogFile returns the path from the repo root
// to the log file in system format
func (pt *PackageTask) RepoRelativeSystemLogFile() string {
	return filepath.Join(pt.Dir, logDir, logFilename(pt.Task))
}

func (pt *PackageTask) packageRelativeSharableLogFile() string {
	return strings.Join([]string{logDir, logFilename(pt.Task)}, "/")
}

func logFilename(taskName string) string {
	escapedTaskName := strings.ReplaceAll(taskName, ":", "$colon$")
	return fmt.Sprintf("turbo-%v.log", escapedTaskName)
}

// OutputPrefix returns the prefix to be used for logging and ui for this task
func (pt *PackageTask) OutputPrefix(isSinglePackage bool) string {
	if isSinglePackage {
		return pt.Task
	}
	return fmt.Sprintf("%v:%v", pt.PackageName, pt.Task)
}

// HashableOutputs returns the package-relative globs for files to be considered outputs
// of this task
func (pt *PackageTask) HashableOutputs() hash.TaskOutputs {
	inclusionOutputs := []string{pt.packageRelativeSharableLogFile()}
	inclusionOutputs = append(inclusionOutputs, pt.TaskDefinition.Outputs.Inclusions...)

	hashable := hash.TaskOutputs{
		Inclusions: inclusionOutputs,
		Exclusions: pt.TaskDefinition.Outputs.Exclusions,
	}
	hashable.Sort()
	return hashable
}
