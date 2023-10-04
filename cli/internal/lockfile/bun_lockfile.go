package lockfile

import (
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// BunLockfile representation of bun lockfile
type BunLockfile struct {
	contents []byte
}

var _ Lockfile = (*BunLockfile)(nil)

// ResolvePackage Given a package and version returns the key, resolved version, and if it was found
func (l *BunLockfile) ResolvePackage(_ turbopath.AnchoredUnixPath, _ string, _ string) (Package, error) {
	// This is only used when doing calculating the transitive deps, but Rust
	// implementations do this calculation on the Rust side.
	panic("Unreachable")
}

// AllDependencies Given a lockfile key return all (dev/optional/peer) dependencies of that package
func (l *BunLockfile) AllDependencies(_ string) (map[string]string, bool) {
	// This is only used when doing calculating the transitive deps, but Rust
	// implementations do this calculation on the Rust side.
	panic("Unreachable")
}

// DecodeBunLockfile Takes the contents of a bun lockfile and returns a struct representation
func DecodeBunLockfile(contents []byte) (*BunLockfile, error) {
	return &BunLockfile{contents: contents}, nil
}

// GlobalChange checks if there are any differences between lockfiles that would completely invalidate
// the cache.
func (l *BunLockfile) GlobalChange(other Lockfile) bool {
	_, ok := other.(*BunLockfile)
	return !ok
}
