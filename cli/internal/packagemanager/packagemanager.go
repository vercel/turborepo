// Adapted from https://github.com/replit/upm
// Copyright (c) 2019 Neoreason d/b/a Repl.it. All rights reserved.
// SPDX-License-Identifier: MIT

package packagemanager

import (
	"fmt"
	"path/filepath"

	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/globby"
	"github.com/vercel/turbo/cli/internal/lockfile"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// PackageManager is an abstraction across package managers
type PackageManager struct {
	// The descriptive name of the Package Manager.
	Name string

	// The unique identifier of the Package Manager.
	Slug string

	// The command used to invoke the Package Manager.
	Command string

	// The location of the package spec file used by the Package Manager.
	Specfile string

	// The location of the package lock file used by the Package Manager.
	Lockfile string

	// The directory in which package assets are stored by the Package Manager.
	PackageDir string

	// The location of the file that defines the workspace. Empty if workspaces defined in package.json
	WorkspaceConfigurationPath string

	// The separator that the Package Manger uses to identify arguments that
	// should be passed through to the underlying script.
	ArgSeparator []string

	// Return the list of workspace glob
	getWorkspaceGlobs func(rootpath turbopath.AbsoluteSystemPath) ([]string, error)

	// Return the list of workspace ignore globs
	getWorkspaceIgnores func(pm PackageManager, rootpath turbopath.AbsoluteSystemPath) ([]string, error)

	// Detect if Turbo knows how to produce a pruned workspace for the project
	canPrune func(cwd turbopath.AbsoluteSystemPath) (bool, error)

	// Read a lockfile for a given package manager
	UnmarshalLockfile func(rootPackageJSON *fs.PackageJSON, contents []byte) (lockfile.Lockfile, error)

	// Prune the given pkgJSON to only include references to the given patches
	prunePatches func(pkgJSON *fs.PackageJSON, patches []turbopath.AnchoredUnixPath) error
}

var packageManagers = []PackageManager{
	nodejsYarn,
	nodejsBerry,
	nodejsNpm,
	nodejsPnpm,
	nodejsPnpm6,
}

// GetPackageManager reads the package manager name sent by the Rust side
func GetPackageManager(name string) (packageManager *PackageManager, err error) {
	switch name {
	case "yarn":
		return &nodejsYarn, nil
	case "berry":
		return &nodejsBerry, nil
	case "npm":
		return &nodejsNpm, nil
	case "pnpm":
		return &nodejsPnpm, nil
	case "pnpm6":
		return &nodejsPnpm6, nil
	default:
		return nil, errors.New("Unknown package manager")
	}
}

// GetWorkspaces returns the list of package.json files for the current repository.
func (pm PackageManager) GetWorkspaces(rootpath turbopath.AbsoluteSystemPath) ([]string, error) {
	globs, err := pm.getWorkspaceGlobs(rootpath)
	if err != nil {
		return nil, err
	}

	justJsons := make([]string, len(globs))
	for i, space := range globs {
		justJsons[i] = filepath.Join(space, "package.json")
	}

	ignores, err := pm.getWorkspaceIgnores(pm, rootpath)
	if err != nil {
		return nil, err
	}

	f, err := globby.GlobFiles(rootpath.ToStringDuringMigration(), justJsons, ignores)
	if err != nil {
		return nil, err
	}

	return f, nil
}

// GetWorkspaceIgnores returns an array of globs not to search for workspaces.
func (pm PackageManager) GetWorkspaceIgnores(rootpath turbopath.AbsoluteSystemPath) ([]string, error) {
	return pm.getWorkspaceIgnores(pm, rootpath)
}

// CanPrune returns if turbo can produce a pruned workspace. Can error if fs issues occur
func (pm PackageManager) CanPrune(projectDirectory turbopath.AbsoluteSystemPath) (bool, error) {
	if pm.canPrune != nil {
		return pm.canPrune(projectDirectory)
	}
	return false, nil
}

// ReadLockfile will read the applicable lockfile into memory
func (pm PackageManager) ReadLockfile(projectDirectory turbopath.AbsoluteSystemPath, rootPackageJSON *fs.PackageJSON) (lockfile.Lockfile, error) {
	if pm.UnmarshalLockfile == nil {
		return nil, nil
	}
	contents, err := projectDirectory.UntypedJoin(pm.Lockfile).ReadFile()
	if err != nil {
		return nil, fmt.Errorf("reading %s: %w", pm.Lockfile, err)
	}
	lf, err := pm.UnmarshalLockfile(rootPackageJSON, contents)
	if err != nil {
		return nil, errors.Wrapf(err, "error in %v", pm.Lockfile)
	}
	return lf, nil
}

// PrunePatchedPackages will alter the provided pkgJSON to only reference the provided patches
func (pm PackageManager) PrunePatchedPackages(pkgJSON *fs.PackageJSON, patches []turbopath.AnchoredUnixPath) error {
	if pm.prunePatches != nil {
		return pm.prunePatches(pkgJSON, patches)
	}
	return nil
}
