package fs

import (
	"io"
	"io/ioutil"
	"log"
	"os"
	"path/filepath"
	"runtime"
	"strings"

	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/util"
)

// https://github.com/thought-machine/please/blob/master/src/fs/fs.go

// DirPermissions are the default permission bits we apply to directories.
const DirPermissions = os.ModeDir | 0775

// EnsureDir ensures that the directory of the given file has been created.
func EnsureDir(filename string) error {
	dir := filepath.Dir(filename)
	err := os.MkdirAll(dir, DirPermissions)
	if err != nil && FileExists(dir) {
		// It looks like this is a file and not a directory. Attempt to remove it; this can
		// happen in some cases if you change a rule from outputting a file to a directory.
		log.Printf("Attempting to remove file %s; a subdirectory is required", dir)
		if err2 := os.Remove(dir); err2 == nil {
			err = os.MkdirAll(dir, DirPermissions)
		} else {
			return err
		}
	}
	return err
}

var nonRelativeSentinel string = ".." + string(filepath.Separator)

// DirContainsPath returns true if the path 'target' is contained within 'dir'
// Expects both paths to be absolute and does not verify that either path exists.
func DirContainsPath(dir string, target string) (bool, error) {
	// On windows, trying to get a relative path between files on different volumes
	// is an error. We don't care about the error, it's good enough for us to say
	// that one path doesn't contain the other if they're on different volumes.
	if runtime.GOOS == "windows" && filepath.VolumeName(dir) != filepath.VolumeName(target) {
		return false, nil
	}
	// In Go, filepath.Rel can return a path that starts with "../" or equivalent.
	// Checking filesystem-level contains can get extremely complicated
	// (see https://github.com/golang/dep/blob/f13583b555deaa6742f141a9c1185af947720d60/internal/fs/fs.go#L33)
	// As a compromise, rely on the stdlib to generate a relative path and then check
	// if the first step is "../".
	rel, err := filepath.Rel(dir, target)
	if err != nil {
		return false, err
	}
	return !strings.HasPrefix(rel, nonRelativeSentinel), nil
}

// PathExists returns true if the given path exists, as a file or a directory.
func PathExists(filename string) bool {
	_, err := os.Lstat(filename)
	return err == nil
}

// FileExists returns true if the given path exists and is a file.
func FileExists(filename string) bool {
	info, err := os.Lstat(filename)
	return err == nil && !info.IsDir()
}

// CopyFile copies a file from 'from' to 'to', with an attempt to perform a copy & rename
// to avoid chaos if anything goes wrong partway.
func CopyFile(from *LstatCachedFile, to string) error {
	fromMode, err := from.GetMode()
	if err != nil {
		return errors.Wrapf(err, "getting mode for %v", from.Path)
	}
	if fromMode&os.ModeSymlink != 0 {
		target, err := from.Path.Readlink()
		if err != nil {
			return errors.Wrapf(err, "reading link target for %v", from.Path)
		}
		if err := EnsureDir(to); err != nil {
			return err
		}
		if _, err := os.Lstat(to); err == nil {
			// target link file exist, should remove it first
			err := os.Remove(to)
			if err != nil {
				return err
			}
		}
		return os.Symlink(target, to)
	}
	fromFile, err := from.Path.Open()
	if err != nil {
		return err
	}
	defer util.CloseAndIgnoreError(fromFile)
	return writeFileFromStream(fromFile, to, fromMode)
}

// writeFileFromStream writes data from a reader to the file named 'to', with an attempt to perform
// a copy & rename to avoid chaos if anything goes wrong partway.
func writeFileFromStream(fromFile io.Reader, to string, mode os.FileMode) error {
	dir, file := filepath.Split(to)
	if dir != "" {
		if err := os.MkdirAll(dir, DirPermissions); err != nil {
			return err
		}
	}
	tempFile, err := ioutil.TempFile(dir, file)
	if err != nil {
		return err
	}
	if _, err := io.Copy(tempFile, fromFile); err != nil {
		return err
	}
	if err := tempFile.Close(); err != nil {
		return err
	}
	// OK, now file is written; adjust permissions appropriately.
	if mode == 0 {
		mode = 0664
	}
	if err := os.Chmod(tempFile.Name(), mode); err != nil {
		return err
	}
	// And move it to its final destination.
	return renameFile(tempFile.Name(), to)
}

// IsDirectory checks if a given path is a directory
func IsDirectory(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.IsDir()
}

// Try to gracefully rename the file as the os.Rename does not work across
// filesystems and on most Linux systems /tmp is mounted as tmpfs
func renameFile(from, to string) (err error) {
	err = os.Rename(from, to)
	if err == nil {
		return nil
	}
	err = copyFile(from, to)
	if err != nil {
		return err
	}
	err = os.RemoveAll(from)
	if err != nil {
		return err
	}
	return nil
}

func copyFile(from, to string) (err error) {
	in, err := os.Open(from)
	if err != nil {
		return err
	}
	defer in.Close()

	out, err := os.Create(to)
	if err != nil {
		return err
	}
	defer func() {
		if e := out.Close(); e != nil {
			err = e
		}
	}()

	_, err = io.Copy(out, in)
	if err != nil {
		return err
	}

	si, err := os.Stat(from)
	if err != nil {
		return err
	}
	err = os.Chmod(to, si.Mode())
	if err != nil {
		return err
	}

	return nil
}
