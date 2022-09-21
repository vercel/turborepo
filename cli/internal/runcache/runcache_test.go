package runcache

import (
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/nodes"
)

func Test_OutputGlobs() {
	pkg := fs.PackageJSON{}
	// We only care about the output globs
	taskDefinition := fs.TaskDefinition{
		Outputs:     []string{".next/**", ".next/cache/**"},
		ShouldCache: true,
	}
	packageCache := nodes.PackageTask{
		TaskID:         "foobar",
		Task:           "build",
		PackageName:    "docs",
		Pkg:            &pkg,
		TaskDefinition: &taskDefinition,
	}
}
