//go:build go || !rust
// +build go !rust

package lockfile

import (
	mapset "github.com/deckarep/golang-set"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// TransitiveClosure the set of all lockfile keys that pkg depends on
func TransitiveClosure(
	workspaceDir turbopath.AnchoredUnixPath,
	unresolvedDeps map[string]string,
	lockFile Lockfile,
) (mapset.Set, error) {
	return transitiveClosure(workspaceDir, unresolvedDeps, lockFile)
}
