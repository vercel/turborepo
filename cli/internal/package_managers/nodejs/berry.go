package nodejs

import (
	"fmt"
	"os/exec"
	"path/filepath"

	"github.com/Masterminds/semver"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/package_managers/api"
	"github.com/vercel/turborepo/cli/internal/util"
)

var NodejsBerry = api.PackageManager{
	Name:       "nodejs-berry",
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

	Matches: func(manager string, version string) (bool, error) {
		if manager != "yarn" {
			return false, nil
		}

		v, err := semver.NewVersion(version)
		if err != nil {
			return false, fmt.Errorf("could not parse yarn version: %w", err)
		}
		c, err := semver.NewConstraint(">=2.0.0")
		if err != nil {
			return false, fmt.Errorf("could not create constraint: %w", err)
		}

		return c.Check(v), nil
	},

	Detect: func(projectDirectory string, pkg *fs.PackageJSON, packageManager *api.PackageManager) (bool, error) {
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

		matches, err := packageManager.Matches(packageManager.Slug, string(out))

		// Short-circuit, definitely not Berry because version number says we're Yarn.
		if !matches {
			return false, nil
		}

		// We're Berry!

		// Check for supported configuration.
		isNMLinker, err := util.IsNMLinker(projectDirectory)

		if err != nil {
			// Failed to read the linker state, so unknown configuration is failure.
			return false, fmt.Errorf("could not check if yarn is using nm-linker: %w", err)
		} else if !isNMLinker {
			// Not using nm-linker, so unsupported configuration.
			return false, fmt.Errorf("only yarn nm-linker is supported")
		}

		// Berry, supported configuration.
		return true, nil
	},
}
