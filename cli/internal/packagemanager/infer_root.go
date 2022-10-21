package packagemanager

import (
	"path/filepath"

	"github.com/vercel/turborepo/cli/internal/doublestar"
	"github.com/vercel/turborepo/cli/internal/turbopath"
)

// PackageType represents the mode in which turbo is running.
type PackageType string

const (
	// Single is for single-package mode.
	Single PackageType = "single"
	// Multi is for monorepo mode.
	Multi PackageType = "multi"
)

func candidateDirectoryWorkspaceGlobs(directory turbopath.AbsoluteSystemPath) []string {
	packageManagers := []PackageManager{
		nodejsNpm,
		nodejsPnpm,
	}

	for _, pm := range packageManagers {
		globs, err := pm.getWorkspaceGlobs(directory)
		if err != nil {
			// Try the other package manager workspace formats.
			continue
		}

		return globs
	}

	return nil
}

func isOneOfTheWorkspaces(globs []string, nearestPackageJSONDir turbopath.AbsoluteSystemPath, currentPackageJSONDir turbopath.AbsoluteSystemPath) bool {
	for _, glob := range globs {
		globpattern := currentPackageJSONDir.UntypedJoin(filepath.FromSlash(glob)).ToString()
		match, _ := doublestar.PathMatch(globpattern, nearestPackageJSONDir.ToString())
		if match {
			return true
		}
	}

	return false
}

// InferRoot identifies which directory we should treat as the root, and which mode
// turbo should be in when operating at that directory.
func InferRoot(directory turbopath.AbsoluteSystemPath) (turbopath.AbsoluteSystemPath, PackageType) {
	// Go doesn't have iterators, so this is very not-elegant.

	// Scenarios:
	// 0. Has a turbo.json but doesn't have a peer package.json. directory + multi
	// 1. Nearest turbo.json, check peer package.json/pnpm-workspace.yaml.
	//    A. Has workspaces, multi package mode.
	//    B. No workspaces, single package mode.
	// 2. If no turbo.json find the closest package.json parent.
	//    A. No parent package.json, default to current behavior.
	//    B. Nearest package.json defines workspaces. Can't be in single-package mode, so we bail. (This could be changed in the future.)
	// 3. Closest package.json does not define workspaces. Traverse toward the root looking for package.jsons.
	//    A. No parent package.json with workspaces. nearestPackageJson + single
	//    B. Stop at the first one that has workspaces.
	//       i. If we are one of the workspaces, directory + multi. (This could be changed in the future.)
	//       ii. If we're not one of the workspaces, nearestPackageJson + single.

	nearestTurboJSON, findTurboJSONErr := directory.Findup("turbo.json")
	if findTurboJSONErr != nil {
		// We didn't find a turbo.json. We're in situation 2 or 3.

		// Unroll the first loop for Scenario 2
		nearestPackageJSON, nearestPackageJSONErr := directory.Findup("package.json")

		// If we fail to find any package.json files we aren't in single package mode.
		// We let things go through our existing failure paths.
		// Scenario 2A.
		if nearestPackageJSONErr != nil {
			return directory, Multi
		}

		// If we find a package.json which has workspaces we aren't in single package mode.
		// We let things go through our existing failure paths.
		// Scenario 2B.
		if candidateDirectoryWorkspaceGlobs(nearestPackageJSON.Dir()) != nil {
			// In a future world we could maybe change this behavior.
			// return nearestPackageJson.Dir(), Multi
			return directory, Multi
		}

		// Scenario 3.
		// Find the nearest package.json that has workspaces.
		// If found _and_ the nearestPackageJson is one of the workspaces, thatPackageJson + multi.
		// Else, nearestPackageJson + single
		cursor := nearestPackageJSON.Dir().UntypedJoin("..")
		for {
			nextPackageJSON, nextPackageJSONErr := cursor.Findup("package.json")
			if nextPackageJSONErr != nil {
				// We haven't found a parent defining workspaces.
				// So we're single package mode at nearestPackageJson.
				// Scenario 3A.
				return nearestPackageJSON.Dir(), Single
			}

			// Found a package.json file, see if it has workspaces.
			// Workspaces are not allowed to be recursive, so we know what to
			// return the moment we find something with workspaces.
			globs := candidateDirectoryWorkspaceGlobs(nextPackageJSON.Dir())
			if globs != nil {
				if isOneOfTheWorkspaces(globs, nearestPackageJSON.Dir(), nextPackageJSON.Dir()) {
					// If it has workspaces, and nearestPackageJson is one of them, we're multi.
					// We don't infer in this scenario.
					// Scenario 3BI.
					// TODO: return nextPackageJson.Dir(), Multi
					return directory, Multi
				}

				// We found a parent with workspaces, but we're not one of them.
				// We choose to operate in single package mode.
				// Scenario 3BII
				return nearestPackageJSON.Dir(), Single
			}

			// Loop around and see if we have another parent.
			cursor = nextPackageJSON.Dir().UntypedJoin("..")
		}
	} else {
		// If there is no sibling package.json we do no inference.
		siblingPackageJSONPath := nearestTurboJSON.Dir().UntypedJoin("package.json")
		if !siblingPackageJSONPath.Exists() {
			// We do no inference.
			// Scenario 0
			return directory, Multi
		}

		if candidateDirectoryWorkspaceGlobs(nearestTurboJSON.Dir()) != nil {
			// Scenario 1A.
			return nearestTurboJSON.Dir(), Multi
		}

		// Scenario 1B.
		return nearestTurboJSON.Dir(), Single
	}
}
