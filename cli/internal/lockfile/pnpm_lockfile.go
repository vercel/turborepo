package lockfile

import (
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

// GlobalChange checks if there are any differences between lockfiles that would completely invalidate
// the cache.
func (p *PnpmLockfile) GlobalChange(other Lockfile) bool {
	o, ok := other.(*PnpmLockfile)
	if !ok {
		return true
	}

	return ffi.GlobalChange("pnpm", o.contents, p.contents)
}
