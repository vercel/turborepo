package lockfile

import (
	"io"

	"github.com/vercel/turbo/cli/internal/ffi"
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

// Subgraph Given a list of lockfile keys returns a Lockfile based off the original one that only contains the packages given
func (l *YarnLockfile) Subgraph(workspacePackages []turbopath.AnchoredSystemPath, packages []string) (Lockfile, error) {
	workspaces := make([]string, len(workspacePackages))
	for i, workspace := range workspacePackages {
		workspaces[i] = workspace.ToUnixPath().ToString()
	}
	contents, err := ffi.Subgraph("yarn", l.contents, workspaces, packages, nil)
	if err != nil {
		return nil, err
	}
	return &YarnLockfile{contents: contents}, nil
}

// Encode encode the lockfile representation and write it to the given writer
func (l *YarnLockfile) Encode(w io.Writer) error {
	_, err := w.Write(l.contents)
	return err
}

// Patches return a list of patches used in the lockfile
func (l *YarnLockfile) Patches() []turbopath.AnchoredUnixPath {
	return nil
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
