//go:build rust
// +build rust

package fs

import (
	"github.com/vercel/turbo/cli/internal/ffi"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// RecursiveCopy copies either a single file or a directory.
func RecursiveCopy(from turbopath.AbsoluteSystemPath, to turbopath.AbsoluteSystemPath) error {
	return ffi.RecursiveCopy(from.ToString(), to.ToString())
}
