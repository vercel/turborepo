package lockfile

import (
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// YarnLockfile representation of yarn lockfile
type YarnLockfile struct {
	contents []byte
}

var _ Lockfile = (*YarnLockfile)(nil)

// ResolvePackage Given a package and version returns the key, resolved version, and if it was found
func (l *YarnLockfile) ResolvePackage(_workspacePath turbopath.AnchoredUnixPath, name string, version string) (Package, error) {
	// This is only used when doing calculating the transitive deps, but Rust
	// implementations do this calculation on the Rust side.
	panic("Unreachable")
}

// AllDependencies Given a lockfile key return all (dev/optional/peer) dependencies of that package
func (l *YarnLockfile) AllDependencies(key string) (map[string]string, bool) {
	// This is only used when doing calculating the transitive deps, but Rust
	// implementations do this calculation on the Rust side.
	panic("Unreachable")
}

// DecodeYarnLockfile Takes the contents of a yarn lockfile and returns a struct representation
func DecodeYarnLockfile(contents []byte) (*YarnLockfile, error) {
	return &YarnLockfile{contents: contents}, nil
}

// GlobalChange checks if there are any differences between lockfiles that would completely invalidate
// the cache.
func (l *YarnLockfile) GlobalChange(other Lockfile) bool {
	_, ok := other.(*YarnLockfile)
	return !ok
}
