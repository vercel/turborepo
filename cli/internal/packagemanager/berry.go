package packagemanager

import (
	"fmt"
	"os/exec"
	"strings"

	"github.com/Masterminds/semver"
	"github.com/pkg/errors"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/lockfile"
	"github.com/vercel/turborepo/cli/internal/turbopath"
	"github.com/vercel/turborepo/cli/internal/util"
)

var nodejsBerry = PackageManager{
	Name:       "nodejs-berry",
	Slug:       "yarn",
	Command:    "yarn",
	Specfile:   "package.json",
	Lockfile:   "yarn.lock",
	PackageDir: "node_modules",

	getWorkspaceGlobs: func(rootpath turbopath.AbsoluteSystemPath) ([]string, error) {
		pkg, err := fs.ReadPackageJSON(rootpath.UntypedJoin("package.json"))
		if err != nil {
			return nil, fmt.Errorf("package.json: %w", err)
		}
		if len(pkg.Workspaces) == 0 {
			return nil, fmt.Errorf("package.json: no workspaces found. Turborepo requires Yarn workspaces to be defined in the root package.json")
		}
		return pkg.Workspaces, nil
	},

	getWorkspaceIgnores: func(pm PackageManager, rootpath turbopath.AbsoluteSystemPath) ([]string, error) {
		// Matches upstream values:
		// Key code: https://github.com/yarnpkg/berry/blob/8e0c4b897b0881878a1f901230ea49b7c8113fbe/packages/yarnpkg-core/sources/Workspace.ts#L64-L70
		return []string{
			"**/node_modules",
			"**/.git",
			"**/.yarn",
		}, nil
	},

	canPrune: func(cwd turbopath.AbsoluteSystemPath) (bool, error) {
		if isNMLinker, err := util.IsNMLinker(cwd.ToStringDuringMigration()); err != nil {
			return false, errors.Wrap(err, "could not determine if yarn is using `nodeLinker: node-modules`")
		} else if !isNMLinker {
			return false, errors.New("only yarn v2/v3 with `nodeLinker: node-modules` is supported at this time")
		}
		return true, nil
	},

	// Versions newer than 2.0 are berry, and before that we simply call them yarn.
	Matches: func(manager string, version string) (bool, error) {
		if manager != "yarn" {
			return false, nil
		}

		v, err := semver.NewVersion(version)
		if err != nil {
			return false, fmt.Errorf("could not parse yarn version: %w", err)
		}
		// -0 allows pre-releases versions to be considered valid
		c, err := semver.NewConstraint(">=2.0.0-0")
		if err != nil {
			return false, fmt.Errorf("could not create constraint: %w", err)
		}

		return c.Check(v), nil
	},

	// Detect for berry needs to identify which version of yarn is running on the system.
	// Further, berry can be configured in an incompatible way, so we check for compatibility here as well.
	detect: func(projectDirectory turbopath.AbsoluteSystemPath, packageManager *PackageManager) (bool, error) {
		specfileExists := projectDirectory.UntypedJoin(packageManager.Specfile).FileExists()
		lockfileExists := projectDirectory.UntypedJoin(packageManager.Lockfile).FileExists()

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

		// See if we're a match when we compare these two things.
		matches, _ := packageManager.Matches(packageManager.Slug, string(out))

		// Short-circuit, definitely not Berry because version number says we're Yarn.
		if !matches {
			return false, nil
		}

		// We're Berry!

		// Check for supported configuration.
		isNMLinker, err := util.IsNMLinker(projectDirectory.ToStringDuringMigration())

		if err != nil {
			// Failed to read the linker state, so we treat an unknown configuration as a failure.
			return false, fmt.Errorf("could not check if yarn is using nm-linker: %w", err)
		} else if !isNMLinker {
			// Not using nm-linker, so unsupported configuration.
			return false, fmt.Errorf("only yarn nm-linker is supported")
		}

		// Berry, supported configuration.
		return true, nil
	},

	readLockfile: func(contents []byte) (lockfile.Lockfile, error) {
		return lockfile.DecodeBerryLockfile(contents)
	},

	prunePatches: func(pkgJSON *fs.PackageJSON, patches []turbopath.AnchoredUnixPath) error {
		pkgJSON.Mu.Lock()
		defer pkgJSON.Mu.Unlock()

		keysToDelete := []string{}
		resolutions, ok := pkgJSON.RawJSON["resolutions"].(map[string]interface{})
		if !ok {
			return fmt.Errorf("Invalid structure for resolutions field in package.json")
		}

		for dependency, untypedPatch := range resolutions {
			inPatches := false
			patch, ok := untypedPatch.(string)
			if !ok {
				return fmt.Errorf("Expected value of %s in package.json to be a string, got %v", dependency, untypedPatch)
			}

			for _, wantedPatch := range patches {
				if strings.HasSuffix(patch, wantedPatch.ToString()) {
					inPatches = true
					break
				}
			}

			// We only want to delete unused patches as they are the only ones that throw if unused
			if !inPatches && strings.HasSuffix(patch, ".patch") {
				keysToDelete = append(keysToDelete, dependency)
			}
		}

		for _, key := range keysToDelete {
			delete(resolutions, key)
		}

		return nil
	},
}
