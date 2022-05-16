package packagemanager

import (
	"fmt"
	"io/ioutil"
	"path/filepath"

	"github.com/vercel/turborepo/cli/internal/fs"
	"gopkg.in/yaml.v3"
)

// PnpmWorkspaces is a representation of workspace package globs found
// in pnpm-workspace.yaml
type PnpmWorkspaces struct {
	Packages []string `yaml:"packages,omitempty"`
}

var nodejsPnpm = PackageManager{
	Name:       "nodejs-pnpm",
	Slug:       "pnpm",
	Command:    "pnpm",
	Specfile:   "package.json",
	Lockfile:   "pnpm-lock.yaml",
	PackageDir: "node_modules",

	getWorkspaceGlobs: func(rootpath string) ([]string, error) {
		bytes, err := ioutil.ReadFile(filepath.Join(rootpath, "pnpm-workspace.yaml"))
		if err != nil {
			return nil, fmt.Errorf("pnpm-workspace.yaml: %w", err)
		}
		var pnpmWorkspaces PnpmWorkspaces
		if err := yaml.Unmarshal(bytes, &pnpmWorkspaces); err != nil {
			return nil, fmt.Errorf("pnpm-workspace.yaml: %w", err)
		}

		if len(pnpmWorkspaces.Packages) == 0 {
			return nil, fmt.Errorf("pnpm-workspace.yaml: no packages found. Turborepo requires pnpm workspaces and thus packages to be defined in the root pnpm-workspace.yaml")
		}

		return pnpmWorkspaces.Packages, nil
	},

	getWorkspaceIgnores: func(pm PackageManager, rootpath string) ([]string, error) {
		// Matches upstream values:
		// function: https://github.com/pnpm/pnpm/blob/d99daa902442e0c8ab945143ebaf5cdc691a91eb/packages/find-packages/src/index.ts#L27
		// key code: https://github.com/pnpm/pnpm/blob/d99daa902442e0c8ab945143ebaf5cdc691a91eb/packages/find-packages/src/index.ts#L30
		// call site: https://github.com/pnpm/pnpm/blob/d99daa902442e0c8ab945143ebaf5cdc691a91eb/packages/find-workspace-packages/src/index.ts#L32-L39
		return []string{
			"**/node_modules/**",
			"**/bower_components/**",
		}, nil
	},

	Matches: func(manager string, version string) (bool, error) {
		return manager == "pnpm", nil
	},

	detect: func(projectDirectory string, packageManager *PackageManager) (bool, error) {
		specfileExists := fs.FileExists(filepath.Join(projectDirectory, packageManager.Specfile))
		lockfileExists := fs.FileExists(filepath.Join(projectDirectory, packageManager.Lockfile))

		return (specfileExists && lockfileExists), nil
	},
}
