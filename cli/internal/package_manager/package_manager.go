// Adapted from https://github.com/replit/upm
// Copyright (c) 2019 Neoreason d/b/a Repl.it. All rights reserved.
// SPDX-License-Identifier: MIT
package package_manager

import (
	"errors"
	"fmt"
	"regexp"
	"strings"

	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/util"
)

// PackageManager is an abstraction across package managers
type PackageManager struct {
	Name       string
	Slug       string
	Command    string
	Specfile   string
	Lockfile   string
	PackageDir string

	// Return the list of workspace glob
	GetWorkspaceGlobs func(rootpath string) ([]string, error)

	Matches func(manager string, version string) (bool, error)

	// Detect if the project is using a specific package manager
	Detect func(projectDirectory string, packageManager *PackageManager) (bool, error)
}

var packageManagers = []PackageManager{
	nodejsYarn,
	nodejsBerry,
	nodejsNpm,
	nodejsPnpm,
}

// ParsePackageManagerString takes a package manager version string parses it into consituent components
func ParsePackageManagerString(packageManager string) (manager string, version string, err error) {
	pattern := `(npm|pnpm|yarn)@(\d+)\.\d+\.\d+(-.+)?`
	re := regexp.MustCompile(pattern)
	match := re.FindString(packageManager)
	if len(match) == 0 {
		return "", "", fmt.Errorf("We could not parse packageManager field in package.json, expected: %s, received: %s", pattern, packageManager)
	}

	return strings.Split(match, "@")[0], strings.Split(match, "@")[1], nil
}

// GetPackageManager attempts all methods for identifying the package manager in use.
func GetPackageManager(projectDirectory string, pkg *fs.PackageJSON) (packageManager *PackageManager, err error) {
	result, _ := readPackageManager(pkg)
	if result != nil {
		return result, nil
	}

	return detectPackageManager(projectDirectory)
}

// readPackageManager attempts to read the package manager from the package.json.
func readPackageManager(pkg *fs.PackageJSON) (packageManager *PackageManager, err error) {
	if pkg.PackageManager != "" {
		manager, version, err := ParsePackageManagerString(pkg.PackageManager)
		if err != nil {
			return nil, err
		}

		for _, packageManager := range packageManagers {
			isResponsible, err := packageManager.Matches(manager, version)
			if isResponsible && (err == nil) {
				return &packageManager, nil
			}
		}
	}

	return nil, errors.New(util.Sprintf("We did not find a package manager specified in your root package.json. Please set the \"packageManager\" property in your root package.json (${UNDERLINE}https://nodejs.org/api/packages.html#packagemanager)${RESET} or run `npx @turbo/codemod add-package-manager` in the root of your monorepo."))
}

// detectPackageManager attempts to detect the package manager by inspecting the project directory state.
func detectPackageManager(projectDirectory string) (packageManager *PackageManager, err error) {
	for _, packageManager := range packageManagers {
		isResponsible, err := packageManager.Detect(projectDirectory, &packageManager)
		if err != nil {
			return nil, err
		}
		if isResponsible {
			return &packageManager, nil
		}
	}

	return nil, errors.New(util.Sprintf("We did not detect an in-use package manager for your project. Please set the \"packageManager\" property in your root package.json (${UNDERLINE}https://nodejs.org/api/packages.html#packagemanager)${RESET} or run `npx @turbo/codemod add-package-manager` in the root of your monorepo."))
}
