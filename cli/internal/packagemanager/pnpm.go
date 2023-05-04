package packagemanager

import (
	"fmt"
	"strings"

	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/lockfile"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/yaml"
)

// PnpmWorkspaces is a representation of workspace package globs found
// in pnpm-workspace.yaml
type PnpmWorkspaces struct {
	Packages []string `yaml:"packages,omitempty"`
}

func readPnpmWorkspacePackages(workspaceFile turbopath.AbsoluteSystemPath) ([]string, error) {
	bytes, err := workspaceFile.ReadFile()
	if err != nil {
		return nil, fmt.Errorf("%v: %w", workspaceFile, err)
	}
	var pnpmWorkspaces PnpmWorkspaces
	if err := yaml.Unmarshal(bytes, &pnpmWorkspaces); err != nil {
		return nil, fmt.Errorf("%v: %w", workspaceFile, err)
	}
	return pnpmWorkspaces.Packages, nil
}

func getPnpmWorkspaceGlobs(rootpath turbopath.AbsoluteSystemPath) ([]string, error) {
	pkgGlobs, err := readPnpmWorkspacePackages(rootpath.UntypedJoin("pnpm-workspace.yaml"))
	if err != nil {
		return nil, err
	}

	if len(pkgGlobs) == 0 {
		return nil, fmt.Errorf("pnpm-workspace.yaml: no packages found. Turborepo requires pnpm workspaces and thus packages to be defined in the root pnpm-workspace.yaml")
	}

	filteredPkgGlobs := []string{}
	for _, pkgGlob := range pkgGlobs {
		if !strings.HasPrefix(pkgGlob, "!") {
			filteredPkgGlobs = append(filteredPkgGlobs, pkgGlob)
		}
	}
	return filteredPkgGlobs, nil
}

func getPnpmWorkspaceIgnores(pm PackageManager, rootpath turbopath.AbsoluteSystemPath) ([]string, error) {
	// Matches upstream values:
	// function: https://github.com/pnpm/pnpm/blob/d99daa902442e0c8ab945143ebaf5cdc691a91eb/packages/find-packages/src/index.ts#L27
	// key code: https://github.com/pnpm/pnpm/blob/d99daa902442e0c8ab945143ebaf5cdc691a91eb/packages/find-packages/src/index.ts#L30
	// call site: https://github.com/pnpm/pnpm/blob/d99daa902442e0c8ab945143ebaf5cdc691a91eb/packages/find-workspace-packages/src/index.ts#L32-L39
	ignores := []string{
		"**/node_modules/**",
		"**/bower_components/**",
	}
	pkgGlobs, err := readPnpmWorkspacePackages(rootpath.UntypedJoin("pnpm-workspace.yaml"))
	if err != nil {
		return nil, err
	}
	for _, pkgGlob := range pkgGlobs {
		if strings.HasPrefix(pkgGlob, "!") {
			ignores = append(ignores, pkgGlob[1:])
		}
	}
	return ignores, nil
}

var nodejsPnpm = PackageManager{
	Name:       "nodejs-pnpm",
	Slug:       "pnpm",
	Command:    "pnpm",
	Specfile:   "package.json",
	Lockfile:   "pnpm-lock.yaml",
	PackageDir: "node_modules",
	// pnpm v7+ changed their handling of '--'. We no longer need to pass it to pass args to
	// the script being run, and in fact doing so will cause the '--' to be passed through verbatim,
	// potentially breaking scripts that aren't expecting it.
	// We are allowed to use nil here because ArgSeparator already has a type, so it's a typed nil,
	// This could just as easily be []string{}, but the style guide says to prefer
	// nil for empty slices.
	ArgSeparator:               nil,
	WorkspaceConfigurationPath: "pnpm-workspace.yaml",

	getWorkspaceGlobs: getPnpmWorkspaceGlobs,

	getWorkspaceIgnores: getPnpmWorkspaceIgnores,

	canPrune: func(cwd turbopath.AbsoluteSystemPath) (bool, error) {
		return true, nil
	},

	UnmarshalLockfile: func(_rootPackageJSON *fs.PackageJSON, contents []byte) (lockfile.Lockfile, error) {
		return lockfile.DecodePnpmLockfile(contents)
	},

	prunePatches: func(pkgJSON *fs.PackageJSON, patches []turbopath.AnchoredUnixPath) error {
		return pnpmPrunePatches(pkgJSON, patches)
	},
}

func pnpmPrunePatches(pkgJSON *fs.PackageJSON, patches []turbopath.AnchoredUnixPath) error {
	pkgJSON.Mu.Lock()
	defer pkgJSON.Mu.Unlock()

	keysToDelete := []string{}
	pnpmConfig, ok := pkgJSON.RawJSON["pnpm"].(map[string]interface{})
	if !ok {
		return fmt.Errorf("Invalid structure for pnpm field in package.json")
	}
	patchedDependencies, ok := pnpmConfig["patchedDependencies"].(map[string]interface{})
	if !ok {
		return fmt.Errorf("Invalid structure for patchedDependencies field in package.json")
	}

	for dependency, untypedPatch := range patchedDependencies {
		patch, ok := untypedPatch.(string)
		if !ok {
			return fmt.Errorf("Expected only strings in patchedDependencies. Got %v", untypedPatch)
		}

		inPatches := false

		for _, wantedPatch := range patches {
			if wantedPatch.ToString() == patch {
				inPatches = true
				break
			}
		}

		if !inPatches {
			keysToDelete = append(keysToDelete, dependency)
		}
	}

	for _, key := range keysToDelete {
		delete(patchedDependencies, key)
	}

	return nil
}
