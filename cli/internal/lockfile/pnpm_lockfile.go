package lockfile

import (
	"io"

	"github.com/vercel/turbo/cli/internal/ffi"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// PnpmLockfile Go representation of the contents of 'pnpm-lock.yaml'
// Reference https://github.com/pnpm/pnpm/blob/main/packages/lockfile-types/src/index.ts
type PnpmLockfile struct {
	contents []byte
}

var _ Lockfile = (*PnpmLockfile)(nil)

// DecodePnpmLockfile parse a pnpm lockfile
func DecodePnpmLockfile(contents []byte) (*PnpmLockfile, error) {
	return &PnpmLockfile{contents: contents}, nil
}

// ResolvePackage Given a package and version returns the key, resolved version, and if it was found
func (p *PnpmLockfile) ResolvePackage(workspacePath turbopath.AnchoredUnixPath, name string, version string) (Package, error) {
	// This is only used when doing calculating the transitive deps, but Rust
	// implementations do this calculation on the Rust side.
	panic("Unreachable")
}

// AllDependencies Given a lockfile key return all (dev/optional/peer) dependencies of that package
func (p *PnpmLockfile) AllDependencies(key string) (map[string]string, bool) {
	// This is only used when doing calculating the transitive deps, but Rust
	// implementations do this calculation on the Rust side.
	panic("Unreachable")
}

// Subgraph Given a list of lockfile keys returns a Lockfile based off the original one that only contains the packages given
func (p *PnpmLockfile) Subgraph(workspacePackages []turbopath.AnchoredSystemPath, packages []string) (Lockfile, error) {
	workspaces := make([]string, len(workspacePackages))
	for i, workspace := range workspacePackages {
		workspaces[i] = workspace.ToUnixPath().ToString()
	}
	contents, err := ffi.Subgraph("pnpm", p.contents, workspaces, packages, nil)
	if err != nil {
		return nil, err
	}
	return &PnpmLockfile{contents: contents}, nil
}

// Encode encode the lockfile representation and write it to the given writer
func (p *PnpmLockfile) Encode(w io.Writer) error {
	_, err := w.Write(p.contents)
	return err
}

// Patches return a list of patches used in the lockfile
func (p *PnpmLockfile) Patches() []turbopath.AnchoredUnixPath {
	rawPatches := ffi.Patches(p.contents, "pnpm")
	if len(rawPatches) == 0 {
		return nil
	}
	patches := make([]turbopath.AnchoredUnixPath, len(rawPatches))
	for i, patch := range rawPatches {
		patches[i] = turbopath.AnchoredUnixPath(patch)
	}
	return patches
}

// GlobalChange checks if there are any differences between lockfiles that would completely invalidate
// the cache.
func (p *PnpmLockfile) GlobalChange(other Lockfile) bool {
	o, ok := other.(*PnpmLockfile)
	if !ok {
		return true
	}

	return ffi.GlobalChange("pnpm", o.contents, p.contents)
}
