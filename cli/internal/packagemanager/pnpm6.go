package packagemanager

import (
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/lockfile"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

const pnpm6Lockfile = "pnpm-lock.yaml"

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
	Lockfile:                   pnpm6Lockfile,
	PackageDir:                 "node_modules",
	ArgSeparator:               func(_userArgs []string) []string { return []string{"--"} },
	WorkspaceConfigurationPath: "pnpm-workspace.yaml",

	getWorkspaceGlobs: getPnpmWorkspaceGlobs,

	getWorkspaceIgnores: getPnpmWorkspaceIgnores,

	canPrune: func(cwd turbopath.AbsoluteSystemPath) (bool, error) {
		return true, nil
	},

	GetLockfileName: func(_ turbopath.AbsoluteSystemPath) string {
		return pnpm6Lockfile
	},

	GetLockfilePath: func(projectDirectory turbopath.AbsoluteSystemPath) turbopath.AbsoluteSystemPath {
		return projectDirectory.UntypedJoin(pnpm6Lockfile)
	},

	GetLockfileContents: func(projectDirectory turbopath.AbsoluteSystemPath) ([]byte, error) {
		return projectDirectory.UntypedJoin(pnpm6Lockfile).ReadFile()
	},

	UnmarshalLockfile: func(_rootPackageJSON *fs.PackageJSON, contents []byte) (lockfile.Lockfile, error) {
		return lockfile.DecodePnpmLockfile(contents)
	},
}
