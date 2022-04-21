package packagemanager

import (
	"fmt"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/Masterminds/semver"
	"github.com/vercel/turborepo/cli/internal/fs"
)

var nodejsYarn = PackageManager{
	Name:       "nodejs-yarn",
	Slug:       "yarn",
	Command:    "yarn",
	Specfile:   "package.json",
	Lockfile:   "yarn.lock",
	PackageDir: "node_modules",

	GetWorkspaceGlobs: func(rootpath string) ([]string, error) {
		pkg, err := fs.ReadPackageJSON(filepath.Join(rootpath, "package.json"))
		if err != nil {
			return nil, fmt.Errorf("package.json: %w", err)
		}
		if len(pkg.Workspaces) == 0 {
			return nil, fmt.Errorf("package.json: no workspaces found. Turborepo requires Yarn workspaces to be defined in the root package.json")
		}
		return pkg.Workspaces, nil
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
		c, err := semver.NewConstraint("<2.0.0")
		if err != nil {
			return false, fmt.Errorf("could not create constraint: %w", err)
		}

		return c.Check(v), nil
	},

	// Detect for yarn needs to identify which version of yarn is running on the system.
	Detect: func(projectDirectory string, packageManager *PackageManager) (bool, error) {
		specfileExists := fs.FileExists(filepath.Join(projectDirectory, packageManager.Specfile))
		lockfileExists := fs.FileExists(filepath.Join(projectDirectory, packageManager.Lockfile))

		// Short-circuit, definitely not Yarn.
		if !specfileExists || !lockfileExists {
			return false, nil
		}

		cmd := exec.Command("yarn", "--version")
		cmd.Dir = projectDirectory
		out, err := cmd.Output()
		if err != nil {
			return false, fmt.Errorf("could not detect yarn version: %w", err)
		}

		return packageManager.Matches(packageManager.Slug, strings.TrimSpace(string(out)))
	},
}
