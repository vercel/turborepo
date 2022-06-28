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

// CopyOrLinkFile either copies or hardlinks a file based on the link argument.
// Falls back to a copy if link fails and fallback is true.
func CopyOrLinkFile(from *LstatCachedFile, to string, link bool, fallback bool) error {
	fromMode, err := from.GetMode()
	if err != nil {
		return err
	}
	if (fromMode & os.ModeSymlink) != 0 {
		// Create an equivalent symlink in the new location.
		dest, err := from.Path.Readlink()
		if err != nil {
			return err
		}
		// Make sure the link we're about to create doesn't already exist
		if err := os.Remove(to); err != nil && !errors.Is(err, os.ErrNotExist) {
			return err
		}
		return os.Symlink(dest, to)
	}
	if link {
		if err := from.Path.Link(to); err == nil || !fallback {
			return err
		}
	}
	return CopyFile(from, to)
}

// RecursiveCopy copies either a single file or a directory.
// 'mode' is the mode of the destination file.
func RecursiveCopy(from string, to string) error {
	return RecursiveCopyOrLinkFile(from, to, false, false)
}

// RecursiveCopyOrLinkFile recursively copies or links a file or directory.
// If 'link' is true then we'll hardlink files instead of copying them.
// If 'fallback' is true then we'll fall back to a copy if linking fails.
func RecursiveCopyOrLinkFile(from string, to string, link bool, fallback bool) error {
	// Verified all callers are passing in absolute paths for from (and to)
	statedFrom := LstatCachedFile{Path: UnsafeToAbsolutePath(from)}
	fromType, err := statedFrom.GetType()
	if err != nil {
		return err
	}

	if fromType.IsDir() {
		return WalkMode(statedFrom.Path.ToStringDuringMigration(), func(name string, isDir bool, fileType os.FileMode) error {
			dest := filepath.Join(to, name[len(statedFrom.Path.ToString()):])
			if isDir {
				return os.MkdirAll(dest, DirPermissions)
			}
			if isSame, err := SameFile(statedFrom.Path.ToStringDuringMigration(), name); err != nil {
				return err
			} else if isSame {
				return nil
			}
			// name is absolute, (originates from godirwalk)
			return CopyOrLinkFile(&LstatCachedFile{Path: UnsafeToAbsolutePath(name), fileType: &fileType}, dest, link, fallback)
		})
	}
	return CopyOrLinkFile(&statedFrom, to, link, fallback)
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

// SameFile returns true if the two given paths refer to the same physical
// file on disk, using the unique file identifiers from the underlying
// operating system. For example, on Unix systems this checks whether the
// two files are on the same device and have the same inode.
func SameFile(a string, b string) (bool, error) {
	if a == b {
		return true, nil
	}

	aInfo, err := os.Lstat(a)
	if err != nil {
		if os.IsNotExist(err) {
			return false, nil
		}
		return false, err
	}

	bInfo, err := os.Lstat(b)
	if err != nil {
		if os.IsNotExist(err) {
			return false, nil
		}
		return false, err
	}

	return os.SameFile(aInfo, bInfo), nil
}
