package packagemanager

import (
	"fmt"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/Masterminds/semver"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/lockfile"
	"github.com/vercel/turborepo/cli/internal/turbopath"
)

var nodejsYarn = PackageManager{
	Name:         "nodejs-yarn",
	Slug:         "yarn",
	Command:      "yarn",
	Specfile:     "package.json",
	Lockfile:     "yarn.lock",
	PackageDir:   "node_modules",
	ArgSeparator: []string{"--"},

	getWorkspaceGlobs: func(rootpath turbopath.AbsoluteSystemPath) ([]string, error) {
		pkg, err := fs.ReadPackageJSON(rootpath.UnsafeJoin("package.json"))
		if err != nil {
			return nil, fmt.Errorf("package.json: %w", err)
		}
		if len(pkg.Workspaces) == 0 {
			return nil, fmt.Errorf("package.json: no workspaces found. Turborepo requires Yarn workspaces to be defined in the root package.json")
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

	// Versions older than 2.0 are yarn, after that they become berry
	Matches: func(manager string, version string) (bool, error) {
		if manager != "yarn" {
			return false, nil
		}

		v, err := semver.NewVersion(version)
		if err != nil {
			return false, fmt.Errorf("could not parse yarn version: %w", err)
		}
		c, err := semver.NewConstraint("<2.0.0-0")
		if err != nil {
			return false, fmt.Errorf("could not create constraint: %w", err)
		}

		return c.Check(v), nil
	},

	// Detect for yarn needs to identify which version of yarn is running on the system.
	detect: func(projectDirectory turbopath.AbsoluteSystemPath, packageManager *PackageManager) (bool, error) {
		specfileExists := projectDirectory.UnsafeJoin(packageManager.Specfile).FileExists()
		lockfileExists := projectDirectory.UnsafeJoin(packageManager.Lockfile).FileExists()

		// Short-circuit, definitely not Yarn.
		if !specfileExists || !lockfileExists {
			return false, nil
		}

		cmd := exec.Command("yarn", "--version")
		cmd.Dir = projectDirectory.ToString()
		out, err := cmd.Output()
		if err != nil {
			return false, fmt.Errorf("could not detect yarn version: %w", err)
		}

		return packageManager.Matches(packageManager.Slug, strings.TrimSpace(string(out)))
	},

	readLockfile: func(contents []byte) (lockfile.Lockfile, error) {
		return lockfile.DecodeYarnLockfile(contents)
	},
}
