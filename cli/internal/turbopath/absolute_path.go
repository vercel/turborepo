package turbopath

import (
	"io/ioutil"
	"log"
	"os"
	"path/filepath"
	"strings"
)

// dirPermissions are the default permission bits we apply to directories.
const dirPermissions = os.ModeDir | 0775

// ensureDir ensures that the directory of the given file has been created.
func ensureDir(filename string) error {
	dir := filepath.Dir(filename)
	err := os.MkdirAll(dir, dirPermissions)
	if err != nil && fileExists(dir) {
		// It looks like this is a file and not a directory. Attempt to remove it; this can
		// happen in some cases if you change a rule from outputting a file to a directory.
		log.Printf("Attempting to remove file %s; a subdirectory is required", dir)
		if err2 := os.Remove(dir); err2 == nil {
			err = os.MkdirAll(dir, dirPermissions)
		} else {
			return err
		}
	}
	return err
}

var nonRelativeSentinel string = ".." + string(filepath.Separator)

// dirContainsPath returns true if the path 'target' is contained within 'dir'
// Expects both paths to be absolute and does not verify that either path exists.
func dirContainsPath(dir string, target string) (bool, error) {
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

// fileExists returns true if the given path exists and is a file.
func fileExists(filename string) bool {
	info, err := os.Lstat(filename)
	return err == nil && !info.IsDir()
}

// AbsolutePath represents a platform-dependent absolute path on the filesystem,
// and is used to enfore correct path manipulation
type AbsolutePath string

func (ap AbsolutePath) ToStringDuringMigration() string {
	return ap.asString()
}

func (ap AbsolutePath) Join(args ...string) AbsolutePath {
	return AbsolutePath(filepath.Join(ap.asString(), filepath.Join(args...)))
}
func (ap AbsolutePath) asString() string {
	return string(ap)
}
func (ap AbsolutePath) Dir() AbsolutePath {
	return AbsolutePath(filepath.Dir(ap.asString()))
}

// MkdirAll implements os.MkdirAll(ap, DirPermissions|0644)
func (ap AbsolutePath) MkdirAll() error {
	return os.MkdirAll(ap.asString(), dirPermissions|0644)
}

// Open implements os.Open(ap) for an absolute path
func (ap AbsolutePath) Open() (*os.File, error) {
	return os.Open(ap.asString())
}

// OpenFile implements os.OpenFile for an absolute path
func (ap AbsolutePath) OpenFile(flags int, mode os.FileMode) (*os.File, error) {
	return os.OpenFile(ap.asString(), flags, mode)
}

func (ap AbsolutePath) FileExists() bool {
	return fileExists(ap.asString())
}

// Lstat implements os.Lstat for absolute path
func (ap AbsolutePath) Lstat() (os.FileInfo, error) {
	return os.Lstat(ap.asString())
}

// DirExists returns true if this path points to a directory
func (ap AbsolutePath) DirExists() bool {
	info, err := ap.Lstat()
	return err == nil && info.IsDir()
}

// ContainsPath returns true if this absolute path is a parent of the
// argument.
func (ap AbsolutePath) ContainsPath(other AbsolutePath) (bool, error) {
	return dirContainsPath(ap.asString(), other.asString())
}

// ReadFile reads the contents of the specified file
func (ap AbsolutePath) ReadFile() ([]byte, error) {
	return ioutil.ReadFile(ap.asString())
}

// WriteFile writes the contents of the specified file
func (ap AbsolutePath) WriteFile(contents []byte, mode os.FileMode) error {
	return ioutil.WriteFile(ap.asString(), contents, mode)
}

// EnsureDir ensures that the directory containing this file exists
func (ap AbsolutePath) EnsureDir() error {
	return ensureDir(ap.asString())
}

// Create is the AbsolutePath wrapper for os.Create
func (ap AbsolutePath) Create() (*os.File, error) {
	return os.Create(ap.asString())
}

// Ext implements filepath.Ext(ap) for an absolute path
func (ap AbsolutePath) Ext() string {
	return filepath.Ext(ap.asString())
}

// ToString returns the string representation of this absolute path. Used for
// interfacing with APIs that require a string
func (ap AbsolutePath) ToString() string {
	return ap.asString()
}

// RelativePathString returns the relative path from this AbsolutePath to another absolute path in string form as a string
func (ap AbsolutePath) RelativePathString(path string) (string, error) {
	return filepath.Rel(ap.asString(), path)
}

// PathTo returns the relative path between two absolute paths
// This should likely eventually return an AnchoredSystemPath
func (ap AbsolutePath) PathTo(other AbsolutePath) (string, error) {
	return ap.RelativePathString(other.asString())
}

// Symlink implements os.Symlink(target, ap) for absolute path
func (ap AbsolutePath) Symlink(target string) error {
	return os.Symlink(target, ap.asString())
}

// Readlink implements os.Readlink(ap) for an absolute path
func (ap AbsolutePath) Readlink() (string, error) {
	return os.Readlink(ap.asString())
}

// Remove removes the file or (empty) directory at the given path
func (ap AbsolutePath) Remove() error {
	return os.Remove(ap.asString())
}

// RemoveAll implements os.RemoveAll for absolute paths.
func (ap AbsolutePath) RemoveAll() error {
	return os.RemoveAll(ap.asString())
}

// Base implements filepath.Base for an absolute path
func (ap AbsolutePath) Base() string {
	return filepath.Base(ap.asString())
}

// Rename implements os.Rename(ap, dest) for absolute paths
func (ap AbsolutePath) Rename(dest AbsolutePath) error {
	return os.Rename(ap.asString(), dest.asString())
}
