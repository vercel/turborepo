// Package lockfile provides the lockfile interface and implementations for the various package managers
package lockfile

import (
	"io"

	"github.com/vercel/turborepo/cli/internal/turbopath"
)

// Lockfile Interface for general operations that work accross all lockfiles
type Lockfile interface {
	// ResolvePackage Given a workspace, a package it imports and version returns the key, resolved version, and if it was found
	ResolvePackage(workspace string, name string, version string) (string, string, bool)
	// AllDependencies Given a lockfile key return all (dev/optional/peer) dependencies of that package
	AllDependencies(key string) (map[string]string, bool)
	// Subgraph Given a list of lockfile keys returns a Lockfile based off the original one that only contains the packages given
	Subgraph(workspacePackages []turbopath.AnchoredSystemPath, packages []string) (Lockfile, error)
	// Encode encode the lockfile representation and write it to the given writer
	Encode(w io.Writer) error
	// Patches return a list of patches used in the lockfile
	Patches() []turbopath.AnchoredUnixPath
}
