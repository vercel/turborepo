package fs

import (
	"os"
	"path/filepath"

	"github.com/karrick/godirwalk"
)

// CopyOrLinkFile either copies or hardlinks a file based on the link argument.
// Falls back to a copy if link fails and fallback is true.
func CopyOrLinkFile(from, to string, fromMode, toMode os.FileMode, link, fallback bool) error {
	if link {
		if (fromMode & os.ModeSymlink) != 0 {
			// Don't try to hard-link to a symlink, that doesn't work reliably across all platforms.
			// Instead recreate an equivalent symlink in the new location.
			dest, err := os.Readlink(from)
			if err != nil {
				return err
			}
			return os.Symlink(dest, to)
		}
		if err := os.Link(from, to); err == nil || !fallback {
			return err
		}
	}
	return CopyFile(from, to, toMode)
}

// RecursiveCopy copies either a single file or a directory.
// 'mode' is the mode of the destination file.
func RecursiveCopy(from string, to string, mode os.FileMode) error {
	return RecursiveCopyOrLinkFile(from, to, mode, false, false)
}

// RecursiveCopyOrLinkFile recursively copies or links a file or directory.
// 'mode' is the mode of the destination file.
// If 'link' is true then we'll hardlink files instead of copying them.
// If 'fallback' is true then we'll fall back to a copy if linking fails.
func RecursiveCopyOrLinkFile(from string, to string, mode os.FileMode, link, fallback bool) error {
	info, err := os.Lstat(from)
	if err != nil {
		return err
	}
	if info.IsDir() {
		return WalkMode(from, func(name string, isDir bool, fileMode os.FileMode) error {
			dest := filepath.Join(to, name[len(from):])
			if isDir {
				return os.MkdirAll(dest, DirPermissions)
			}
			if isSame, err := SameFile(from, name); err != nil {
				return err
			} else if isSame {
				return nil
			}
			return CopyOrLinkFile(name, dest, fileMode, mode, link, fallback)
		})
	}
	return CopyOrLinkFile(from, to, info.Mode(), mode, link, fallback)
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
			if isDirLike, err := info.IsDirOrSymlinkToDir(); err == nil {
				return callback(name, isDirLike, info.ModeType())
			} else {
				// Skip running callback on "dead" symlink (symlink to directory that doesn't exist)
				if err, ok := err.(*os.PathError); !ok {
					return err
				}
				return nil
			}
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
func SameFile(a, b string) (bool, error) {
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
