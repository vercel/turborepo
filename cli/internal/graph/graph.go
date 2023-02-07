// Package graph contains the CompleteGraph struct and some methods around it
package graph

import (
	gocontext "context"
	"fmt"
	"path/filepath"

	"github.com/pyr-sh/dag"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/nodes"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/util"
)

// WorkspaceInfos holds information about each workspace in the monorepo.
type WorkspaceInfos map[string]*fs.PackageJSON

// CompleteGraph represents the common state inferred from the filesystem and pipeline.
// It is not intended to include information specific to a particular run.
type CompleteGraph struct {
	// WorkspaceGraph expresses the dependencies between packages
	WorkspaceGraph dag.AcyclicGraph

	// Pipeline is config from turbo.json
	Pipeline fs.Pipeline

	// WorkspaceInfos stores the package.json contents by package name
	WorkspaceInfos WorkspaceInfos

	// GlobalHash is the hash of all global dependencies
	GlobalHash string

	RootNode string

	// Map of TaskDefinitions by taskID
	TaskDefinitions map[string]*fs.TaskDefinition

	RepoRoot turbopath.AbsoluteSystemPath
}

// GetPackageTaskVisitor wraps a `visitor` function that is used for walking the TaskGraph
// during execution (or dry-runs). The function returned here does not execute any tasks itself,
// but it helps curry some data from the Complete Graph and pass it into the visitor function.
func (g *CompleteGraph) GetPackageTaskVisitor(ctx gocontext.Context, visitor func(ctx gocontext.Context, packageTask *nodes.PackageTask) error) func(taskID string) error {
	return func(taskID string) error {
		packageName, taskName := util.GetPackageTaskFromId(taskID)
		pkg, ok := g.WorkspaceInfos[packageName]
		if !ok {
			return fmt.Errorf("cannot find package %v for task %v", packageName, taskID)
		}

		taskDefinition, ok := g.TaskDefinitions[taskID]
		if !ok {
			return fmt.Errorf("Could not find definition for task")
		}

		packageTask := &nodes.PackageTask{
			TaskID:          taskID,
			Task:            taskName,
			PackageName:     packageName,
			Pkg:             pkg,
			Dir:             pkg.Dir.ToString(),
			TaskDefinition:  taskDefinition,
			Outputs:         taskDefinition.Outputs.Inclusions,
			ExcludedOutputs: taskDefinition.Outputs.Exclusions,
		}

		if cmd, ok := pkg.Scripts[taskName]; ok {
			packageTask.Command = cmd
		}

		packageTask.LogFile = repoRelativeLogFile(packageTask)

		return visitor(ctx, packageTask)
	}
}

// repoRelativeLogFile returns the path to the log file for this task execution as a
// relative path from the root of the monorepo.
func repoRelativeLogFile(pt *nodes.PackageTask) string {
	return filepath.Join(pt.Pkg.Dir.ToStringDuringMigration(), ".turbo", fmt.Sprintf("turbo-%v.log", pt.Task))
}
