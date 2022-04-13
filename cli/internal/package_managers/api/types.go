package api

import "github.com/vercel/turborepo/cli/internal/fs"

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
	Detect func(string, *fs.PackageJSON, *PackageManager) (bool, error)
}
