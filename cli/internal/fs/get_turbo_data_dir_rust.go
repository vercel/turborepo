//go:build rust
// +build rust

package fs

import (
	"github.com/vercel/turbo/cli/internal/ffi"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// GetTurboDataDir returns a directory outside of the repo
// where turbo can store data files related to turbo.
func GetTurboDataDir() turbopath.AbsoluteSystemPath {
	dir := ffi.GetTurboDataDir()
	return turbopath.AbsoluteSystemPathFromUpstream(dir)
}
