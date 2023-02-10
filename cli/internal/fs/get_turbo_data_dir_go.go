//go:build go || !rust
// +build go !rust

package fs

import (
	"github.com/adrg/xdg"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// GetTurboDataDir returns a directory outside of the repo
// where turbo can store data files related to turbo.
func GetTurboDataDir() turbopath.AbsoluteSystemPath {
	dataHome := AbsoluteSystemPathFromUpstream(xdg.DataHome)
	return dataHome.UntypedJoin("turborepo")
}
