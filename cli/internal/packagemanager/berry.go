package packagemanager

import (
	"fmt"
	"strings"

	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/lockfile"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/util"
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

	UnmarshalLockfile: func(rootPackageJSON *fs.PackageJSON, contents []byte) (lockfile.Lockfile, error) {
		var resolutions map[string]string
		if untypedResolutions, ok := rootPackageJSON.RawJSON["resolutions"]; ok {
			if untypedResolutions, ok := untypedResolutions.(map[string]interface{}); ok {
				resolutions = make(map[string]string, len(untypedResolutions))
				for resolution, reference := range untypedResolutions {
					if reference, ok := reference.(string); ok {
						resolutions[resolution] = reference
					}
				}
			}
		}

		return lockfile.DecodeBerryLockfile(contents, resolutions)
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
