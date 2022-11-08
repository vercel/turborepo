// Adapted from https://github.com/thought-machine/please
// Copyright Thought Machine, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
package fs

import (
	"errors"
	"os"
	"path/filepath"

	"github.com/karrick/godirwalk"
)

// RecursiveCopy copies either a single file or a directory.
// 'mode' is the mode of the destination file.
func RecursiveCopy(from string, to string) error {
	// Verified all callers are passing in absolute paths for from (and to)
	statedFrom := LstatCachedFile{Path: UnsafeToAbsoluteSystemPath(from)}
	fromType, err := statedFrom.GetType()
	if err != nil {
		return err
	}

	if fromType.IsDir() {
		return WalkMode(statedFrom.Path.ToStringDuringMigration(), func(name string, isDir bool, fileType os.FileMode) error {
			dest := filepath.Join(to, name[len(statedFrom.Path.ToString()):])
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
	return CopyFile(&statedFrom, to)
}

// Walk implements an equivalent to filepath.Walk.
// It's implemented over github.com/karrick/godirwalk but the provided interface doesn't use that
// to make it a little easier to handle.
func Walk(rootPath string, callback func(name string, isDir bool) error) error {
	return WalkMode(rootPath, func(name string, isDir bool, mode os.FileMode) error {
		return callback(name, isDir)
	})
}

// WalkMode is like Walk but the callback receives an additional type specifying the file mode type.
// N.B. This only includes the bits of the mode that determine the mode type, not the permissions.
func WalkMode(rootPath string, callback func(name string, isDir bool, mode os.FileMode) error) error {
	return godirwalk.Walk(rootPath, &godirwalk.Options{
		Callback: func(name string, info *godirwalk.Dirent) error {
			// currently we support symlinked files, but not symlinked directories:
			// For copying, we Mkdir and bail if we encounter a symlink to a directoy
			// For finding packages, we enumerate the symlink, but don't follow inside
			isDir, err := info.IsDirOrSymlinkToDir()
			if err != nil {
				pathErr := &os.PathError{}
				if errors.As(err, &pathErr) {
					// If we have a broken link, skip this entry
					return godirwalk.SkipThis
				}
				return err
			}
			return callback(name, isDir, info.ModeType())
		},
		ErrorCallback: func(pathname string, err error) godirwalk.ErrorAction {
			pathErr := &os.PathError{}
			if errors.As(err, &pathErr) {
				return godirwalk.SkipNode
			}
			return godirwalk.Halt
		},
		Unsorted:            true,
		AllowNonDirectory:   true,
		FollowSymbolicLinks: false,
	})
}
