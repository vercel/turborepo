package lockfile

import (
	"io"

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

// Subgraph Given a list of lockfile keys returns a Lockfile based off the original one that only contains the packages given
func (l *NpmLockfile) Subgraph(workspacePackages []turbopath.AnchoredSystemPath, packages []string) (Lockfile, error) {
	workspaces := make([]string, len(workspacePackages))
	for i, workspace := range workspacePackages {
		workspaces[i] = workspace.ToUnixPath().ToString()
	}
	contents, err := ffi.Subgraph("npm", l.contents, workspaces, packages, nil)
	if err != nil {
		return nil, err
	}
	return &NpmLockfile{contents: contents}, nil
}

// Encode the lockfile representation and write it to the given writer
func (l *NpmLockfile) Encode(w io.Writer) error {
	_, err := w.Write(l.contents)
	return err
}

// Patches return a list of patches used in the lockfile
func (l *NpmLockfile) Patches() []turbopath.AnchoredUnixPath {
	return nil
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
