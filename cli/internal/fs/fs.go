package fs

import (
	"fmt"
	"io"
	"log"
	"os"
	"path/filepath"
	"strings"
	"turbo/internal/util"

	"github.com/bmatcuk/doublestar"
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

// IsSymlink returns true if the given path exists and is a symlink.
func IsSymlink(filename string) bool {
	info, err := os.Lstat(filename)
	return err == nil && (info.Mode()&os.ModeSymlink) != 0
}

// CopyFile copies a file from 'from' to 'to', with an attempt to perform a copy & rename
// to avoid chaos if anything goes wrong partway.
func CopyFile(from string, to string, mode os.FileMode) error {
	fromFile, err := os.Open(from)
	if err != nil {
		return err
	}
	defer fromFile.Close()

	dir, _ := filepath.Split(to)
	if dir != "" {
		if err := os.MkdirAll(dir, DirPermissions); err != nil {
			return err
		}
	}
	// Set permissions properly
	if mode == 0 {
		mode = 0664
	}
	toFile, err := os.OpenFile(to, 0302, mode)
	if err != nil {
		return err
	}
	if _, err := io.Copy(toFile, fromFile); err != nil {
		os.Remove(to)
		return err
	}
	toFile.Close()
	return nil
}

// IsDirectory checks if a given path is a directory
func IsDirectory(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.IsDir()
}

// IsPackage returns true if the given directory name is a package (i.e. contains a build file)
func IsPackage(buildFileNames []string, name string) bool {
	for _, buildFileName := range buildFileNames {
		if FileExists(filepath.Join(name, buildFileName)) {
			return true
		}
	}
	return false
}

// GlobList accepts a list of doublestar directive globs and returns a list of files matching them
func Globby(globs []string) ([]string, error) {
	var fileset = make(util.Set)
	for _, output := range globs {
		results, err := doublestar.Glob(strings.TrimPrefix(output, "!"))
		if err != nil {
			return nil, fmt.Errorf("invalid glob %v: %w", output, err)
		}
		// we handle negation via "!" by removing the result from the fileset
		for _, result := range results {
			if strings.HasPrefix(output, "!") {
				fileset.Delete(result)
			} else {
				fileset.Add(result)
			}
		}
	}
	return fileset.UnsafeListOfStrings(), nil
}
