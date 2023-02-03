// TODO add back build tags for Rust

package lockfile

import (
	"io"

	mapset "github.com/deckarep/golang-set"
	"github.com/vercel/turbo/cli/internal/ffi"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

type NpmLockfileRust struct {
	// We just story the entire lockfile in memory and pass it for every call
	contents []byte
}

func (l *NpmLockfileRust) ResolvePackage(workspacePath turbopath.AnchoredUnixPath, name string, version string) (Package, error) {
	panic("UNUSED")
}

func (l *NpmLockfileRust) AllDependencies(key string) (map[string]string, bool) {
	panic("UNUSED")
}

func (l *NpmLockfileRust) Subgraph(workspacePackages []turbopath.AnchoredSystemPath, packages []string) (Lockfile, error) {
	workspaces := make([]string, len(workspacePackages))
	for i, workspace := range workspacePackages {
		workspaces[i] = workspace.ToUnixPath().ToString()
	}
	contents, err := ffi.NpmSubgraph(l.contents, workspaces, packages)
	if err != nil {
		return nil, err
	}
	return &NpmLockfileRust{contents: contents}, nil
}

func (l *NpmLockfileRust) Encode(w io.Writer) error {
	// do we need to check num of bytes written?
	_, err := w.Write(l.contents)
	return err
}

func (l *NpmLockfileRust) Patches() []turbopath.AnchoredUnixPath {
	return nil
}

func (l *NpmLockfileRust) GlobalChange(other Lockfile) bool {
	// TODO we can probably just parse the json and look for known global changes
	return false
}

var _ (Lockfile) = (*NpmLockfileRust)(nil)

func DecodeRustNpmLockfile(contents []byte) (Lockfile, error) {
	return &NpmLockfileRust{contents: contents}, nil
}

func NpmTransitiveDeps(lockfile *NpmLockfileRust, workspacePath turbopath.AnchoredUnixPath, unresolvedDeps map[string]string) (mapset.Set, error) {
	// we convert pkg to
	pkgDir := workspacePath.ToString()

	packages, err := ffi.NpmTransitiveDeps(lockfile.contents, pkgDir, unresolvedDeps)
	if err != nil {
		return nil, err
	}

	deps := make([]interface{}, len(packages))
	for i, pkg := range packages {
		deps[i] = Package{
			Found:   pkg.Found,
			Key:     pkg.Key,
			Version: pkg.Version,
		}
	}

	return mapset.NewSetFromSlice(deps), nil
}
