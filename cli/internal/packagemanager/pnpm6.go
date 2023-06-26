package packagemanager

import (
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/lockfile"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// Pnpm6Workspaces is a representation of workspace package globs found
// in pnpm-workspace.yaml
type Pnpm6Workspaces struct {
	Packages []string `yaml:"packages,omitempty"`
}

var nodejsPnpm6 = PackageManager{
	Name:                       "nodejs-pnpm6",
	Slug:                       "pnpm",
	Command:                    "pnpm",
	Specfile:                   "package.json",
	Lockfile:                   "pnpm-lock.yaml",
	PackageDir:                 "node_modules",
	ArgSeparator:               []string{"--"},
	WorkspaceConfigurationPath: "pnpm-workspace.yaml",

	getWorkspaceGlobs: getPnpmWorkspaceGlobs,

	getWorkspaceIgnores: getPnpmWorkspaceIgnores,

	canPrune: func(cwd turbopath.AbsoluteSystemPath) (bool, error) {
		return true, nil
	},

	UnmarshalLockfile: func(_rootPackageJSON *fs.PackageJSON, contents []byte) (lockfile.Lockfile, error) {
		return lockfile.DecodePnpmLockfile(contents)
	},
}
