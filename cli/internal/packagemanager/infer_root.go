package packagemanager

import (
	"github.com/vercel/turborepo/cli/internal/doublestar"
	"github.com/vercel/turborepo/cli/internal/turbopath"
)

type PackageType string

const (
	Single PackageType = "single"
	Multi  PackageType = "multi"
)

func candidateDirectoryHasWorkspaces(directory turbopath.AbsoluteSystemPath) bool {
	packageManagers := []PackageManager{
		nodejsNpm,
		nodejsPnpm,
	}

	for _, pm := range packageManagers {
		_, err := pm.getWorkspaceGlobs(directory)
		if err != nil {
			// Try the other package manager workspace formats.
			continue
		}

		return true
	}

	return false
}

func isOneOfTheWorkspaces(nearestPackageJsonDir turbopath.AbsoluteSystemPath, currentPackageJsonDir turbopath.AbsoluteSystemPath) bool {
	packageManagers := []PackageManager{
		nodejsNpm,
		nodejsPnpm,
	}

	for _, pm := range packageManagers {
		globs, err := pm.getWorkspaceGlobs(currentPackageJsonDir)
		if err != nil {
			// Try the other package manager workspace formats.
			continue
		}

		for _, glob := range globs {
			match, _ := doublestar.Match(glob, nearestPackageJsonDir.ToString())
			if match {
				return true
			}
		}
	}

	return false
}

func InferRoot(directory turbopath.AbsoluteSystemPath) (turbopath.AbsoluteSystemPath, PackageType) {
	// Go doesn't have iterators, so this is very not-elegant.

	// Scenarios:
	// 1. Nearest turbo.json, check peer package.json/pnpm-workspace.yaml.
	//    A. Has workspaces, multi package mode.
	//    B. No workspaces, single package mode.
	// 2. If no turbo.json find the closest package.json parent.
	//    A. No parent package.json, default to current behavior.
	//    B. Nearest package.json defines workspaces. Set root to that directory and multi to true.
	// 3. Closest package.json does not define workspaces. Traverse toward the root looking for package.jsons.
	//    A. No parent package.json with workspaces. nearestPackageJson + single
	//    B. Stop at the first one that has workspaces.
	//       i. If we are one of the workspaces, nextPackageJson + multi.
	//       ii. If we're not one of the workspaces, nearestPackageJson + single.

	nearestTurbo, findTurboErr := directory.Findup("turbo.json")
	if findTurboErr != nil {
		// We didn't find a turbo.json. We're in situation 2 or 3.
		nearestPackageJson, nearestPackageJsonErr := directory.Findup("package.json")

		// If we fail to find any package.json files we aren't in single package mode.
		// We let things go through our existing failure paths.
		if nearestPackageJsonErr != nil {
			// Scenario 2A.
			return directory, Multi
		}

		if candidateDirectoryHasWorkspaces(nearestPackageJson.Dir()) {
			// Scenario 2B.
			return nearestPackageJson.Dir(), Multi
		} else {
			// Scenario 3.
			// Find the nearest package.json that has workspaces.
			// If found _and_ the nearestPackageJson is one of the workspaces, thatPackageJson + multi.
			// Else, nearestPackageJson + single
			cursor := nearestPackageJson.UntypedJoin("..", "..")
			for {
				nextPackageJson, nextPackageJsonErr := cursor.Findup("package.json")
				if nextPackageJsonErr != nil {
					// We haven't found a parent defining workspaces.
					// So we're single package mode at nearestPackageJson.
					// Scenario 3A.
					return nearestPackageJson.Dir(), Single
				} else {
					// Found a package.json file, see if it has workspaces.
					// Workspaces are not allowed to be recursive, so we know what to
					// return the moment we find something with workspaces.
					if candidateDirectoryHasWorkspaces(nextPackageJson.Dir()) {
						if isOneOfTheWorkspaces(nearestPackageJson.Dir(), nextPackageJson.Dir()) {
							// If it has workspaces, and nearestPackageJson is one of them, we're multi
							// Scenario 3BI.
							return nextPackageJson.Dir(), Multi
						} else {
							// We found a parent with workspaces, but we're not one of them.
							// We choose to operate in single package mode.
							// Scenario 3BII
							return nearestPackageJson.Dir(), Single
						}
					} else {
						// Loop around and see if we have another parent.
						cursor = nextPackageJson.UntypedJoin("..", "..")
					}
				}
			}
		}
	} else {
		if candidateDirectoryHasWorkspaces(nearestTurbo.Dir()) {
			// Scenario 1A.
			return nearestTurbo.Dir(), Multi
		} else {
			// Scenario 1B.
			return nearestTurbo.Dir(), Single
		}
	}
}
