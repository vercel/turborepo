package lockfile

import (
	"github.com/vercel/turbo/cli/internal/ffi"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// BerryLockfile representation of berry lockfile
type BerryLockfile struct {
	contents    []byte
	resolutions map[string]string
}

// BerryDependencyMetaEntry Structure for holding if a package is optional or not
type BerryDependencyMetaEntry struct {
	Optional  bool `yaml:"optional,omitempty"`
	Unplugged bool `yaml:"unplugged,omitempty"`
}

var _ Lockfile = (*BerryLockfile)(nil)

// ResolvePackage Given a package and version returns the key, resolved version, and if it was found
func (l *BerryLockfile) ResolvePackage(_workspace turbopath.AnchoredUnixPath, name string, version string) (Package, error) {
	panic("Should use Rust implementation")
}

// AllDependencies Given a lockfile key return all (dev/optional/peer) dependencies of that package
func (l *BerryLockfile) AllDependencies(key string) (map[string]string, bool) {
	panic("Should use Rust implementation")
}

// DecodeBerryLockfile Takes the contents of a berry lockfile and returns a struct representation
func DecodeBerryLockfile(contents []byte, resolutions map[string]string) (*BerryLockfile, error) {
	return &BerryLockfile{contents: contents, resolutions: resolutions}, nil
}

// GlobalChange checks if there are any differences between lockfiles that would completely invalidate
// the cache.
func (l *BerryLockfile) GlobalChange(other Lockfile) bool {
	o, ok := other.(*BerryLockfile)
	if !ok {
		return true
	}

	return ffi.GlobalChange("berry", o.contents, l.contents)
}
