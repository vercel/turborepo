// Package nodes defines the nodes that are present in the execution graph used by turbo.
package nodes

import (
	"fmt"

	"github.com/vercel/turbo/cli/internal/fs"
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
	LogFile         string
	Hash            string
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
func (pt *PackageTask) HashableOutputs() fs.TaskOutputs {
	inclusionOutputs := []string{fmt.Sprintf(".turbo/turbo-%v.log", pt.Task)}
	inclusionOutputs = append(inclusionOutputs, pt.TaskDefinition.Outputs.Inclusions...)

	return fs.TaskOutputs{
		Inclusions: inclusionOutputs,
		Exclusions: pt.TaskDefinition.Outputs.Exclusions,
	}
}
