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
	"github.com/vercel/turborepo/cli/internal/package_manager/api"
	"github.com/vercel/turborepo/cli/internal/package_manager/nodejs"
	"github.com/vercel/turborepo/cli/internal/util"
)

var packageManagers = []api.PackageManager{
	nodejs.NodejsYarn,
	nodejs.NodejsBerry,
	nodejs.NodejsNpm,
	nodejs.NodejsPnpm,
}

func ParsePackageManagerString(packageManager string) (string, string, error) {
	pattern := `(npm|pnpm|yarn)@(\d+)\.\d+\.\d+(-.+)?`
	re := regexp.MustCompile(pattern)
	match := re.FindString(packageManager)
	if len(match) == 0 {
		return "", "", fmt.Errorf("could not parse packageManager field in package.json, expected: %s, received: %s", pattern, packageManager)
	}

	return strings.Split(match, "@")[0], strings.Split(match, "@")[1], nil
}

func GetPackageManager(projectDirectory string, pkg *fs.PackageJSON) (*api.PackageManager, error) {
	// Attempt to read it.
	if pkg.PackageManager != "" {
		manager, version, err := ParsePackageManagerString(pkg.PackageManager)
		if err == nil {
			for _, packageManager := range packageManagers {
				isResponsible, err := packageManager.Matches(manager, version)
				if isResponsible && (err == nil) {
					return &packageManager, nil
				}
			}
		}
	}

	// Attempt to detect it.
	for _, packageManager := range packageManagers {
		detected, err := packageManager.Detect(projectDirectory, &packageManager)
		if err != nil {
			return nil, err
		}
		if detected {
			return &packageManager, nil
		}
	}

	return nil, errors.New(util.Sprintf("could not detect package manager. Please set the \"api.packageManager\" property in your root package.json (${UNDERLINE}https://nodejs.org/api/packages.html#api.packagemanager)${RESET} or run `npx @turbo/codemod add-package-manager` in the root of your monorepo."))
}
