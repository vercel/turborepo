package turbopath

import (
	"io/ioutil"
	"log"
	"os"
	"path/filepath"
	"strings"
)

// AbsoluteSystemPath is a root-relative path using system separators.
type AbsoluteSystemPath string

// ToString returns a string represenation of this Path.
// Used for interfacing with APIs that require a string.
func (p AbsoluteSystemPath) ToString() string {
	return string(p)
}

// RelativeTo calculates the relative path between two `AbsoluteSystemPath`s.
func (p AbsoluteSystemPath) RelativeTo(basePath AbsoluteSystemPath) (AnchoredSystemPath, error) {
	processed, err := filepath.Rel(basePath.ToString(), p.ToString())
	return AnchoredSystemPath(processed), err
}

// Join appends relative path segments to this AbsoluteSystemPath.
func (p AbsoluteSystemPath) Join(additional ...RelativeSystemPath) AbsoluteSystemPath {
	cast := RelativeSystemPathArray(additional)
	return AbsoluteSystemPath(filepath.Join(p.ToString(), filepath.Join(cast.ToStringArray()...)))
}

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

func (p AbsoluteSystemPath) ToStringDuringMigration() string {
	return p.asString()
}

func (p AbsoluteSystemPath) UnsafeJoin(args ...string) AbsoluteSystemPath {
	return AbsoluteSystemPath(filepath.Join(p.asString(), filepath.Join(args...)))
}
func (p AbsoluteSystemPath) asString() string {
	return string(p)
}
func (p AbsoluteSystemPath) Dir() AbsoluteSystemPath {
	return AbsoluteSystemPath(filepath.Dir(p.asString()))
}

// MkdirAll implements os.MkdirAll(p, DirPermissions|0644)
func (p AbsoluteSystemPath) MkdirAll() error {
	return os.MkdirAll(p.asString(), dirPermissions|0644)
}

// Open implements os.Open(p) for an absolute path
func (p AbsoluteSystemPath) Open() (*os.File, error) {
	return os.Open(p.asString())
}

// OpenFile implements os.OpenFile for an absolute path
func (p AbsoluteSystemPath) OpenFile(flags int, mode os.FileMode) (*os.File, error) {
	return os.OpenFile(p.asString(), flags, mode)
}

func (p AbsoluteSystemPath) FileExists() bool {
	return fileExists(p.asString())
}

// Lstat implements os.Lstat for absolute path
func (p AbsoluteSystemPath) Lstat() (os.FileInfo, error) {
	return os.Lstat(p.asString())
}

// DirExists returns true if this path points to a directory
func (p AbsoluteSystemPath) DirExists() bool {
	info, err := p.Lstat()
	return err == nil && info.IsDir()
}

// ContainsPath returns true if this absolute path is a parent of the
// argument.
func (p AbsoluteSystemPath) ContainsPath(other AbsoluteSystemPath) (bool, error) {
	return dirContainsPath(p.asString(), other.asString())
}

// ReadFile reads the contents of the specified file
func (p AbsoluteSystemPath) ReadFile() ([]byte, error) {
	return ioutil.ReadFile(p.asString())
}

// WriteFile writes the contents of the specified file
func (p AbsoluteSystemPath) WriteFile(contents []byte, mode os.FileMode) error {
	return ioutil.WriteFile(p.asString(), contents, mode)
}

// EnsureDir ensures that the directory containing this file exists
func (p AbsoluteSystemPath) EnsureDir() error {
	return ensureDir(p.asString())
}

// Create is the AbsolutePath wrapper for os.Create
func (p AbsoluteSystemPath) Create() (*os.File, error) {
	return os.Create(p.asString())
}

// Ext implements filepath.Ext(p) for an absolute path
func (p AbsoluteSystemPath) Ext() string {
	return filepath.Ext(p.asString())
}

// RelativePathString returns the relative path from this AbsolutePath to another absolute path in string form as a string
func (p AbsoluteSystemPath) RelativePathString(path string) (string, error) {
	return filepath.Rel(p.asString(), path)
}

// PathTo returns the relative path between two absolute paths
// This should likely eventually return an AnchoredSystemPath
func (p AbsoluteSystemPath) PathTo(other AbsoluteSystemPath) (string, error) {
	return p.RelativePathString(other.asString())
}

// Symlink implements os.Symlink(target, p) for absolute path
func (p AbsoluteSystemPath) Symlink(target string) error {
	return os.Symlink(target, p.asString())
}

// Readlink implements os.Readlink(p) for an absolute path
func (p AbsoluteSystemPath) Readlink() (string, error) {
	return os.Readlink(p.asString())
}

// Remove removes the file or (empty) directory at the given path
func (p AbsoluteSystemPath) Remove() error {
	return os.Remove(p.asString())
}

// RemoveAll implements os.RemoveAll for absolute paths.
func (p AbsoluteSystemPath) RemoveAll() error {
	return os.RemoveAll(p.asString())
}

// Base implements filepath.Base for an absolute path
func (p AbsoluteSystemPath) Base() string {
	return filepath.Base(p.asString())
}

// Rename implements os.Rename(p, dest) for absolute paths
func (p AbsoluteSystemPath) Rename(dest AbsoluteSystemPath) error {
	return os.Rename(p.asString(), dest.asString())
}

// EvalSymlinks implements filepath.EvalSymlinks for absolute path
func (p AbsoluteSystemPath) EvalSymlinks() (AbsoluteSystemPath, error) {
	result, err := filepath.EvalSymlinks(p.asString())
	if err != nil {
		return "", err
	}
	return AbsoluteSystemPath(result), nil
}
