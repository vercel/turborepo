package turbopath

import (
	"io/ioutil"
	"os"
	"path/filepath"
	"strings"
)

// AbsoluteSystemPath is a root-relative path using system separators.
type AbsoluteSystemPath string

// _dirPermissions are the default permission bits we apply to directories.
const _dirPermissions = os.ModeDir | 0775

// _nonRelativeSentinel is the leading sentinel that indicates traversal.
const _nonRelativeSentinel = ".." + string(filepath.Separator)

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

// ToStringDuringMigration returns a string representation of this path.
// These instances should eventually be removed.
func (p AbsoluteSystemPath) ToStringDuringMigration() string {
	return p.ToString()
}

// UntypedJoin is a Join that does not constrain the type of the arguments.
// This enables you to pass in strings, but does not protect you from garbage in.
func (p AbsoluteSystemPath) UntypedJoin(args ...string) AbsoluteSystemPath {
	return AbsoluteSystemPath(filepath.Join(p.ToString(), filepath.Join(args...)))
}

// Dir implements filepath.Dir() for an AbsoluteSystemPath
func (p AbsoluteSystemPath) Dir() AbsoluteSystemPath {
	return AbsoluteSystemPath(filepath.Dir(p.ToString()))
}

// Mkdir implements os.Mkdir(p, perm)
func (p AbsoluteSystemPath) Mkdir(perm os.FileMode) error {
	return os.Mkdir(p.ToString(), perm)
}

// MkdirAll implements os.MkdirAll(p, perm)
func (p AbsoluteSystemPath) MkdirAll(perm os.FileMode) error {
	return os.MkdirAll(p.ToString(), perm)
}

// Open implements os.Open(p) for an AbsoluteSystemPath
func (p AbsoluteSystemPath) Open() (*os.File, error) {
	return os.Open(p.ToString())
}

// OpenFile implements os.OpenFile for an absolute path
func (p AbsoluteSystemPath) OpenFile(flags int, mode os.FileMode) (*os.File, error) {
	return os.OpenFile(p.ToString(), flags, mode)
}

// Lstat implements os.Lstat for absolute path
func (p AbsoluteSystemPath) Lstat() (os.FileInfo, error) {
	return os.Lstat(p.ToString())
}

// Stat implements os.Stat for absolute path
func (p AbsoluteSystemPath) Stat() (os.FileInfo, error) {
	return os.Stat(p.ToString())
}

// Findup checks all parent directories for a file.
func (p AbsoluteSystemPath) Findup(name RelativeSystemPath) (AbsoluteSystemPath, error) {
	path, err := FindupFrom(name.ToString(), p.ToString())

	return AbsoluteSystemPath(path), err

}

// Exists returns true if the given path exists.
func (p AbsoluteSystemPath) Exists() bool {
	_, err := p.Lstat()
	return err == nil
}

// DirExists returns true if the given path exists and is a directory.
func (p AbsoluteSystemPath) DirExists() bool {
	info, err := p.Lstat()
	return err == nil && info.IsDir()
}

// FileExists returns true if the given path exists and is a file.
func (p AbsoluteSystemPath) FileExists() bool {
	info, err := os.Lstat(p.ToString())
	return err == nil && !info.IsDir()
}

// ContainsPath returns true if this absolute path is a parent of the
// argument.
func (p AbsoluteSystemPath) ContainsPath(other AbsoluteSystemPath) (bool, error) {
	// In Go, filepath.Rel can return a path that starts with "../" or equivalent.
	// Checking filesystem-level contains can get extremely complicated
	// (see https://github.com/golang/dep/blob/f13583b555deaa6742f141a9c1185af947720d60/internal/fs/fs.go#L33)
	// As a compromise, rely on the stdlib to generate a relative path and then check
	// if the first step is "../".
	rel, err := filepath.Rel(p.ToString(), other.ToString())
	if err != nil {
		return false, err
	}
	return !strings.HasPrefix(rel, _nonRelativeSentinel), nil
}

// ReadFile reads the contents of the specified file
func (p AbsoluteSystemPath) ReadFile() ([]byte, error) {
	return ioutil.ReadFile(p.ToString())
}

// VolumeName returns the volume of the specified path
func (p AbsoluteSystemPath) VolumeName() string {
	return filepath.VolumeName(p.ToString())
}

// WriteFile writes the contents of the specified file
func (p AbsoluteSystemPath) WriteFile(contents []byte, mode os.FileMode) error {
	return ioutil.WriteFile(p.ToString(), contents, mode)
}

// EnsureDir ensures that the directory containing this file exists
func (p AbsoluteSystemPath) EnsureDir() error {
	dir := p.Dir()
	err := os.MkdirAll(dir.ToString(), _dirPermissions)
	if err != nil && dir.FileExists() {
		// It looks like this is a file and not a directory. Attempt to remove it; this can
		// happen in some cases if you change a rule from outputting a file to a directory.
		if err2 := dir.Remove(); err2 == nil {
			err = os.MkdirAll(dir.ToString(), _dirPermissions)
		} else {
			return err
		}
	}
	return err
}

// MkdirAllMode Create directory at path and all necessary parents ensuring that path has the correct mode set
func (p AbsoluteSystemPath) MkdirAllMode(mode os.FileMode) error {
	info, err := p.Lstat()
	if err == nil {
		if info.IsDir() && info.Mode() == mode {
			// Dir exists with the correct mode
			return nil
		} else if info.IsDir() {
			// Dir exists with incorrect mode
			return os.Chmod(p.ToString(), mode)
		} else {
			// Path exists as file, remove it
			if err := p.Remove(); err != nil {
				return err
			}
		}
	}
	if err := os.MkdirAll(p.ToString(), mode); err != nil {
		return err
	}
	// This is necessary only when umask results in creating a directory with permissions different than the one passed by the user
	return os.Chmod(p.ToString(), mode)
}

// Create is the AbsoluteSystemPath wrapper for os.Create
func (p AbsoluteSystemPath) Create() (*os.File, error) {
	return os.Create(p.ToString())
}

// Ext implements filepath.Ext(p) for an absolute path
func (p AbsoluteSystemPath) Ext() string {
	return filepath.Ext(p.ToString())
}

// RelativePathString returns the relative path from this AbsoluteSystemPath to another absolute path in string form as a string
func (p AbsoluteSystemPath) RelativePathString(path string) (string, error) {
	return filepath.Rel(p.ToString(), path)
}

// PathTo returns the relative path between two absolute paths
// This should likely eventually return an AnchoredSystemPath
func (p AbsoluteSystemPath) PathTo(other AbsoluteSystemPath) (string, error) {
	return p.RelativePathString(other.ToString())
}

// Symlink implements os.Symlink(target, p) for absolute path
func (p AbsoluteSystemPath) Symlink(target string) error {
	return os.Symlink(target, p.ToString())
}

// Readlink implements os.Readlink(p) for an absolute path
func (p AbsoluteSystemPath) Readlink() (string, error) {
	return os.Readlink(p.ToString())
}

// Remove removes the file or (empty) directory at the given path
func (p AbsoluteSystemPath) Remove() error {
	return os.Remove(p.ToString())
}

// RemoveAll implements os.RemoveAll for absolute paths.
func (p AbsoluteSystemPath) RemoveAll() error {
	return os.RemoveAll(p.ToString())
}

// Base implements filepath.Base for an absolute path
func (p AbsoluteSystemPath) Base() string {
	return filepath.Base(p.ToString())
}

// Rename implements os.Rename(p, dest) for absolute paths
func (p AbsoluteSystemPath) Rename(dest AbsoluteSystemPath) error {
	return os.Rename(p.ToString(), dest.ToString())
}

// EvalSymlinks implements filepath.EvalSymlinks for absolute path
func (p AbsoluteSystemPath) EvalSymlinks() (AbsoluteSystemPath, error) {
	result, err := filepath.EvalSymlinks(p.ToString())
	if err != nil {
		return "", err
	}
	return AbsoluteSystemPath(result), nil
}

// HasPrefix is strings.HasPrefix for paths, ensuring that it matches on separator boundaries.
// This does NOT perform Clean in advance.
func (p AbsoluteSystemPath) HasPrefix(prefix AbsoluteSystemPath) bool {
	prefixLen := len(prefix)
	pathLen := len(p)

	if prefixLen > pathLen {
		// Can't be a prefix if longer.
		return false
	} else if prefixLen == pathLen {
		// Can be a prefix if they're equal, but otherwise no.
		return p == prefix
	}

	// otherPath is definitely shorter than p.
	// We need to confirm that p[len(otherPath)] is a system separator.

	return strings.HasPrefix(p.ToString(), prefix.ToString()) && os.IsPathSeparator(p[prefixLen])
}
