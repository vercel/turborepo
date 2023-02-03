// Package lockfile provides the lockfile interface and implementations for the various package managers
package lockfile

import (
	"io"
	"reflect"
	"sort"

	"github.com/vercel/turbo/cli/internal/turbopath"
)

// Lockfile Interface for general operations that work across all lockfiles
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
	// GlobalChange checks if there are any differences between lockfiles that would completely invalidate
	// the cache.
	GlobalChange(other Lockfile) bool
}

// IsNil checks if lockfile is nil
func IsNil(l Lockfile) bool {
	return l == nil || reflect.ValueOf(l).IsNil()
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

// ByKey sort package structures by key
type ByKey []Package

func (p ByKey) Len() int {
	return len(p)
}

func (p ByKey) Swap(i, j int) {
	p[i], p[j] = p[j], p[i]
}

func (p ByKey) Less(i, j int) bool {
	return p[i].Key < p[j].Key
}

var _ (sort.Interface) = (*ByKey)(nil)
