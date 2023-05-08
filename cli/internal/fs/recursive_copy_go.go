//go:build go || !rust
// +build go !rust

package fs

import (
	"os"
	"path/filepath"

	"github.com/vercel/turbo/cli/internal/turbopath"
)

// RecursiveCopy copies either a single file or a directory.
func RecursiveCopy(from turbopath.AbsoluteSystemPath, to turbopath.AbsoluteSystemPath) error {
	// Verified all callers are passing in absolute paths for from (and to)
	statedFrom := LstatCachedFile{Path: from}
	fromType, err := statedFrom.GetType()
	if err != nil {
		return err
	}

	if fromType.IsDir() {
		return WalkMode(statedFrom.Path.ToStringDuringMigration(), func(name string, isDir bool, fileType os.FileMode) error {
			dest := filepath.Join(to.ToStringDuringMigration(), name[len(statedFrom.Path.ToString()):])
			// name is absolute, (originates from godirwalk)
			src := LstatCachedFile{Path: UnsafeToAbsoluteSystemPath(name), fileType: &fileType}
			if isDir {
				mode, err := src.GetMode()
				if err != nil {
					return err
				}
				return os.MkdirAll(dest, mode)
			}
			return CopyFile(&src, dest)
		})
	}
	return CopyFile(&statedFrom, to.ToStringDuringMigration())
}
