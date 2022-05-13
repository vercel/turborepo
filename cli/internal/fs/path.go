package fs

import (
	"fmt"
	"os"
	"path/filepath"
	"syscall"

	"github.com/pkg/errors"
	"github.com/spf13/afero"
)

// AbsolutePath represents a platform-dependent absolute path on the filesystem,
// and is used to enfore correct path manipulation
type AbsolutePath string

func CheckedToAbsolutePath(s string) (AbsolutePath, error) {
	if filepath.IsAbs(s) {
		return AbsolutePath(s), nil
	}
	return "", fmt.Errorf("%v is not an absolute path", s)
}

func UnsafeToAbsolutePath(s string) AbsolutePath {
	return AbsolutePath(s)
}

func GetCwd() (AbsolutePath, error) {
	cwdRaw, err := os.Getwd()
	if err != nil {
		return "", fmt.Errorf("invalid working directory: %w", err)
	}
	cwd, err := CheckedToAbsolutePath(cwdRaw)
	if err != nil {
		return "", fmt.Errorf("cwd is not an absolute path %v: %v", cwdRaw, err)
	}
	return cwd, nil
}

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
func (ap AbsolutePath) MkdirAll() error {
	return os.MkdirAll(ap.asString(), DirPermissions)
}
func (ap AbsolutePath) Open() (*os.File, error) {
	return os.Open(ap.asString())
}

func (ap AbsolutePath) FileExists() bool {
	return FileExists(ap.asString())
}

// ToString returns the string representation of this absolute path. Used for
// interfacing with APIs that require a string
func (ap AbsolutePath) ToString() string {
	return ap.asString()
}

// EnsureDirFS ensures that the directory containing the given filename is created
func EnsureDirFS(fs afero.Fs, filename AbsolutePath) error {
	dir := filename.Dir()
	err := fs.MkdirAll(dir.asString(), DirPermissions)
	if errors.Is(err, syscall.ENOTDIR) {
		err = fs.Remove(dir.asString())
		if err != nil {
			return errors.Wrapf(err, "removing existing file at %v before creating directories", dir)
		}
		err = fs.MkdirAll(dir.asString(), DirPermissions)
		if err != nil {
			return err
		}
	} else if err != nil {
		return errors.Wrapf(err, "creating directories at %v", dir)
	}
	return nil
}

// WriteFile writes the given bytes to the specified file
func WriteFile(fs afero.Fs, filename AbsolutePath, toWrite []byte, mode os.FileMode) error {
	return afero.WriteFile(fs, filename.asString(), toWrite, mode)
}

// ReadFile reads the contents of the specified file
func ReadFile(fs afero.Fs, filename AbsolutePath) ([]byte, error) {
	return afero.ReadFile(fs, filename.asString())
}

// RemoveFile removes the file at the given path
func RemoveFile(fs afero.Fs, filename AbsolutePath) error {
	return fs.Remove(filename.asString())
}
