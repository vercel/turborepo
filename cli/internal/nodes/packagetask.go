// package nodes defines the nodes that are present in the execution graph
// used by turbo.

package nodes

import (
	"fmt"
	"path/filepath"

	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/inference"
	"github.com/vercel/turbo/cli/internal/util"
)

// PackageTask represents running a particular task in a particular package
type PackageTask struct {
	TaskID         string
	Task           string
	PackageName    string
	Pkg            *fs.PackageJSON
	TaskDefinition *fs.TaskDefinition
}

// Command returns the script for this task from package.json and a boolean indicating
// whether or not it exists
func (pt *PackageTask) Command() (string, bool) {
	cmd, ok := pt.Pkg.Scripts[pt.Task]
	return cmd, ok
}

// OutputPrefix returns the prefix to be used for logging and ui for this task
func (pt *PackageTask) OutputPrefix(isSinglePackage bool) string {
	if isSinglePackage {
		return pt.Task
	}
	return fmt.Sprintf("%v:%v", pt.PackageName, pt.Task)
}

// RepoRelativeLogFile returns the path to the log file for this task execution as a
// relative path from the root of the monorepo.
func (pt *PackageTask) RepoRelativeLogFile() string {
	return filepath.Join(pt.Pkg.Dir.ToStringDuringMigration(), ".turbo", fmt.Sprintf("turbo-%v.log", pt.Task))
}

// HashableOutputs returns the package-relative globs for files to be considered outputs
// of this task
func (pt *PackageTask) HashableOutputs() fs.TaskOutputs {
	framework := inference.InferFramework(pt.Pkg)

	inclusionOutputs := util.SetFromStrings(pt.TaskDefinition.Outputs.Inclusions)
	// Automatically include task logs as outputs
	inclusionOutputs.Add(fmt.Sprintf(".turbo/turbo-%v.log", pt.Task))
	for _, configInclusion := range pt.TaskDefinition.Outputs.Inclusions {
		inclusionOutputs.Add(configInclusion)
	}
	if framework != nil {
		for _, frameworkInclusion := range framework.Inclusions {
			inclusionOutputs.Add(frameworkInclusion)
		}
	}

	exclusionOutputs := util.SetFromStrings(pt.TaskDefinition.Outputs.Exclusions)
	if framework != nil {
		for _, frameworkExclusion := range framework.Exclusions {
			exclusionOutputs.Add(frameworkExclusion)
		}
	}

	return fs.TaskOutputs{
		Inclusions: inclusionOutputs.UnsafeListOfStrings(),
		Exclusions: exclusionOutputs.UnsafeListOfStrings(),
	}
}
