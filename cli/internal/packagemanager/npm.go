package packagemanager

import (
	"fmt"
	"path/filepath"

	"github.com/vercel/turborepo/cli/internal/fs"
)

var nodejsNpm = PackageManager{
	Name:       "nodejs-npm",
	Slug:       "npm",
	Command:    "npm",
	Specfile:   "package.json",
	Lockfile:   "package-lock.json",
	PackageDir: "node_modules",

	GetWorkspaceGlobs: func(rootpath string) ([]string, error) {
		pkg, err := fs.ReadPackageJSON(filepath.Join(rootpath, "package.json"))
		if err != nil {
			return nil, fmt.Errorf("package.json: %w", err)
		}
		if len(pkg.Workspaces) == 0 {
			return nil, fmt.Errorf("package.json: no workspaces found. Turborepo requires npm workspaces to be defined in the root package.json")
		}
		return pkg.Workspaces, nil
	},

	Matches: func(manager string, version string) (bool, error) {
		return manager == "npm", nil
	},

	Detect: func(projectDirectory string, packageManager *PackageManager) (bool, error) {
		specfileExists := fs.FileExists(filepath.Join(projectDirectory, packageManager.Specfile))
		lockfileExists := fs.FileExists(filepath.Join(projectDirectory, packageManager.Lockfile))

		return (specfileExists && lockfileExists), nil
	},
}
