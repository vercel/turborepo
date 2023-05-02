package packagemanager

import (
	"errors"
	"fmt"
	"path/filepath"

	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/lockfile"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// NoWorkspacesFoundError is a custom error used so that upstream implementations can switch on it
type NoWorkspacesFoundError struct{}

func (e *NoWorkspacesFoundError) Error() string {
	return "package.json: no workspaces found. Turborepo requires Yarn workspaces to be defined in the root package.json"
}

var nodejsYarn = PackageManager{
	Name:         "nodejs-yarn",
	Slug:         "yarn",
	Command:      "yarn",
	Specfile:     "package.json",
	Lockfile:     "yarn.lock",
	PackageDir:   "node_modules",
	ArgSeparator: []string{"--"},

	getWorkspaceGlobs: func(rootpath turbopath.AbsoluteSystemPath) ([]string, error) {
		pkg, err := fs.ReadPackageJSON(rootpath.UntypedJoin("package.json"))
		if err != nil {
			return nil, fmt.Errorf("package.json: %w", err)
		}
		if len(pkg.Workspaces) == 0 {
			return nil, &NoWorkspacesFoundError{}
		}
		return pkg.Workspaces, nil
	},

	getWorkspaceIgnores: func(pm PackageManager, rootpath turbopath.AbsoluteSystemPath) ([]string, error) {
		// function: https://github.com/yarnpkg/yarn/blob/3119382885ea373d3c13d6a846de743eca8c914b/src/config.js#L799

		// Yarn is unique in ignore patterns handling.
		// The only time it does globbing is for package.json or yarn.json and it scopes the search to each workspace.
		// For example: `apps/*/node_modules/**/+(package.json|yarn.json)`
		// The `extglob` `+(package.json|yarn.json)` (from micromatch) after node_modules/** is redundant.

		globs, err := pm.getWorkspaceGlobs(rootpath)
		if err != nil {
			// In case of a non-monorepo, the workspaces field is empty and only node_modules in the root should be ignored
			var e *NoWorkspacesFoundError
			if errors.As(err, &e) {
				return []string{"node_modules/**"}, nil
			}

			return nil, err
		}

		ignores := make([]string, len(globs))

		for i, glob := range globs {
			ignores[i] = filepath.Join(glob, "/node_modules/**")
		}

		return ignores, nil
	},

	canPrune: func(cwd turbopath.AbsoluteSystemPath) (bool, error) {
		return true, nil
	},

	UnmarshalLockfile: func(_rootPackageJSON *fs.PackageJSON, contents []byte) (lockfile.Lockfile, error) {
		return lockfile.DecodeYarnLockfile(contents)
	},
}
