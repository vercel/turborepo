package lockfile

import (
	"github.com/vercel/turbo/cli/internal/ffi"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// NpmLockfile representation of package-lock.json
type NpmLockfile struct {
	// We just story the entire lockfile in memory and pass it for every call
	contents []byte
}

// ResolvePackage Given a workspace, a package it imports and version returns the key, resolved version, and if it was found
func (l *NpmLockfile) ResolvePackage(workspacePath turbopath.AnchoredUnixPath, name string, version string) (Package, error) {
	// This is only used when doing calculating the transitive deps, but Rust
	// implementations do this calculation on the Rust side.
	panic("Unreachable")
}

// AllDependencies Given a lockfile key return all (dev/optional/peer) dependencies of that package
func (l *NpmLockfile) AllDependencies(key string) (map[string]string, bool) {
	// This is only used when doing calculating the transitive deps, but Rust
	// implementations do this calculation on the Rust side.
	panic("Unreachable")
}

// GlobalChange checks if there are any differences between lockfiles that would completely invalidate
// the cache.
func (l *NpmLockfile) GlobalChange(other Lockfile) bool {
	o, ok := other.(*NpmLockfile)
	if !ok {
		return true
	}

	return ffi.GlobalChange("npm", o.contents, l.contents)
}

var _ (Lockfile) = (*NpmLockfile)(nil)

// DecodeNpmLockfile Parse contents of package-lock.json into NpmLockfile
func DecodeNpmLockfile(contents []byte) (Lockfile, error) {
	return &NpmLockfile{contents: contents}, nil
}
