package lockfile

import (
	"io"

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

// Subgraph Given a list of lockfile keys returns a Lockfile based off the original one that only contains the packages given
func (l *BerryLockfile) Subgraph(workspacePackages []turbopath.AnchoredSystemPath, packages []string) (Lockfile, error) {
	workspaces := make([]string, len(workspacePackages))
	for i, workspace := range workspacePackages {
		workspaces[i] = workspace.ToUnixPath().ToString()
	}
	contents, err := ffi.Subgraph("berry", l.contents, workspaces, packages, l.resolutions)
	if err != nil {
		return nil, err
	}
	return &BerryLockfile{contents: contents, resolutions: l.resolutions}, nil
}

// Encode encode the lockfile representation and write it to the given writer
func (l *BerryLockfile) Encode(w io.Writer) error {
	_, err := w.Write(l.contents)
	return err
}

// Patches return a list of patches used in the lockfile
func (l *BerryLockfile) Patches() []turbopath.AnchoredUnixPath {
	rawPatches := ffi.Patches(l.contents, "berry")
	if len(rawPatches) == 0 {
		return nil
	}
	patches := make([]turbopath.AnchoredUnixPath, len(rawPatches))
	for i, patch := range rawPatches {
		patches[i] = turbopath.AnchoredUnixPath(patch)
	}
	return patches
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
