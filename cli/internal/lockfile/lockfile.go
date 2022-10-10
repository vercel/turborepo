// Package lockfile provides the lockfile interface and implementations for the various package managers
package lockfile

import (
	"io"

	"github.com/vercel/turborepo/cli/internal/turbopath"
)

// Lockfile Interface for general operations that work accross all lockfiles
type Lockfile interface {
	// ResolvePackage Given a workspace, a package it imports and version returns the key, resolved version, and if it was found
	ResolvePackage(workspacePath turbopath.AnchoredUnixPath, name string, version string) (Package, error)
	// AllDependencies Given a lockfile key return all (dev/optional/peer) dependencies of that package
	AllDependencies(key string) (map[string]string, bool)
	// Subgraph Given a list of lockfile keys returns a Lockfile based off the original one that only contains the packages given
	Subgraph(workspacePackages []turbopath.AnchoredSystemPath, packages []string) (Lockfile, error)
	// Encode encode the lockfile representation and write it to the given writer
	Encode(w io.Writer) error
	// Patches return a list of patches used in the lockfile
	Patches() []turbopath.AnchoredUnixPath
}

// Package Structure representing a possible Pack
type Package struct {
	// Key used to lookup a package in the lockfile
	Key string
	// The resolved version of a package as it appears in the lockfile
	Version string
	// Set to true iff Key and Version are set
	Found bool
}
