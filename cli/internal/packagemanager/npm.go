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

	getWorkspaceGlobs: func(rootpath string) ([]string, error) {
		pkg, err := fs.ReadPackageJSON(filepath.Join(rootpath, "package.json"))
		if err != nil {
			return nil, fmt.Errorf("package.json: %w", err)
		}
		if len(pkg.Workspaces) == 0 {
			return nil, fmt.Errorf("package.json: no workspaces found. Turborepo requires npm workspaces to be defined in the root package.json")
		}
		return pkg.Workspaces, nil
	},

	getWorkspaceIgnores: func(pm PackageManager, rootpath string) ([]string, error) {
		// Matches upstream values:
		// function: https://github.com/npm/map-workspaces/blob/a46503543982cb35f51cc2d6253d4dcc6bca9b32/lib/index.js#L73
		// key code: https://github.com/npm/map-workspaces/blob/a46503543982cb35f51cc2d6253d4dcc6bca9b32/lib/index.js#L90-L96
		// call site: https://github.com/npm/cli/blob/7a858277171813b37d46a032e49db44c8624f78f/lib/workspaces/get-workspaces.js#L14
		return []string{
			"**/node_modules/**",
		}, nil
	},

	Matches: func(manager string, version string) (bool, error) {
		return manager == "npm", nil
	},

	detect: func(projectDirectory string, packageManager *PackageManager) (bool, error) {
		specfileExists := fs.FileExists(filepath.Join(projectDirectory, packageManager.Specfile))
		lockfileExists := fs.FileExists(filepath.Join(projectDirectory, packageManager.Lockfile))

		return (specfileExists && lockfileExists), nil
	},
}
