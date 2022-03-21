package fs

import (
	"fmt"
	"io/ioutil"
	"os"
	"path/filepath"
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
func (ap AbsolutePath) Remove() error {
	return os.Remove(ap.asString())
}
func (ap AbsolutePath) Open() (*os.File, error) {
	return os.Open(ap.asString())
}
func (ap AbsolutePath) ReadFile() ([]byte, error) {
	return ioutil.ReadFile(ap.asString())
}

// func (ap AbsolutePath) RepoRel(other AbsolutePath) (RepoRelativePath, error) {
// 	rel, err := filepath.Rel(ap.asString(), other.asString())
// 	if err != nil {
// 		return "", err
// 	}
// 	return RepoRelativePath(rel), nil
// }

// type RepoRelativePath string
// type PackageRelativePath string
