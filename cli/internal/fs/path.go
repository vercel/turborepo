package fs

import (
	"fmt"
	iofs "io/fs"
	"os"
	"path/filepath"
	"reflect"

	"github.com/adrg/xdg"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// CheckedToAbsoluteSystemPath inspects a string and determines if it is an absolute path.
func CheckedToAbsoluteSystemPath(s string) (turbopath.AbsoluteSystemPath, error) {
	if filepath.IsAbs(s) {
		return turbopath.AbsoluteSystemPath(s), nil
	}
	return "", fmt.Errorf("%v is not an absolute path", s)
}

// ResolveUnknownPath returns unknown if it is an absolute path, otherwise, it
// assumes unknown is a path relative to the given root.
func ResolveUnknownPath(root turbopath.AbsoluteSystemPath, unknown string) turbopath.AbsoluteSystemPath {
	if filepath.IsAbs(unknown) {
		return turbopath.AbsoluteSystemPath(unknown)
	}
	return root.UntypedJoin(unknown)
}

// UnsafeToAbsoluteSystemPath directly converts a string to an AbsoluteSystemPath
func UnsafeToAbsoluteSystemPath(s string) turbopath.AbsoluteSystemPath {
	return turbopath.AbsoluteSystemPath(s)
}

// UnsafeToAnchoredSystemPath directly converts a string to an AbsoluteSystemPath
func UnsafeToAnchoredSystemPath(s string) turbopath.AnchoredSystemPath {
	return turbopath.AnchoredSystemPath(s)
}

// AbsoluteSystemPathFromUpstream is used to mark return values from APIs that we
// expect to give us absolute paths. No checking is performed.
// Prefer to use this over a cast to maintain the search-ability of interfaces
// into and out of the turbopath.AbsoluteSystemPath type.
func AbsoluteSystemPathFromUpstream(s string) turbopath.AbsoluteSystemPath {
	return turbopath.AbsoluteSystemPath(s)
}

// GetCwd returns the calculated working directory after traversing symlinks.
func GetCwd(cwdRaw string) (turbopath.AbsoluteSystemPath, error) {
	if cwdRaw == "" {
		var err error
		cwdRaw, err = os.Getwd()
		if err != nil {
			return "", err
		}
	}
	// We evaluate symlinks here because the package managers
	// we support do the same.
	cwdRaw, err := filepath.EvalSymlinks(cwdRaw)
	if err != nil {
		return "", fmt.Errorf("evaluating symlinks in cwd: %w", err)
	}
	cwd, err := CheckedToAbsoluteSystemPath(cwdRaw)
	if err != nil {
		return "", fmt.Errorf("cwd is not an absolute path %v: %v", cwdRaw, err)
	}
	return cwd, nil
}

// GetVolumeRoot returns the root directory given an absolute path.
func GetVolumeRoot(absolutePath string) string {
	return filepath.VolumeName(absolutePath) + string(os.PathSeparator)
}

// CreateDirFSAtRoot creates an `os.dirFS` instance at the root of the
// volume containing the specified path.
func CreateDirFSAtRoot(absolutePath string) iofs.FS {
	return os.DirFS(GetVolumeRoot(absolutePath))
}

// GetDirFSRootPath returns the root path of a os.dirFS.
func GetDirFSRootPath(fsys iofs.FS) string {
	// We can't typecheck fsys to enforce using an `os.dirFS` because the
	// type isn't exported from `os`. So instead, reflection. 🤷‍♂️

	fsysType := reflect.TypeOf(fsys).Name()
	if fsysType != "dirFS" {
		// This is not a user error, fail fast
		panic("GetDirFSRootPath must receive an os.dirFS")
	}

	// The underlying type is a string; this is the original path passed in.
	return reflect.ValueOf(fsys).String()
}

// IofsRelativePath calculates a `os.dirFS`-friendly path from an absolute system path.
func IofsRelativePath(fsysRoot string, absolutePath string) (string, error) {
	return filepath.Rel(fsysRoot, absolutePath)
}

// TempDir returns the absolute path of a directory with the given name
// under the system's default temp directory location
func TempDir(subDir string) turbopath.AbsoluteSystemPath {
	return turbopath.AbsoluteSystemPath(os.TempDir()).UntypedJoin(subDir)
}

// GetUserConfigDir returns the platform-specific common location
// for configuration files that belong to a user.
func GetUserConfigDir() turbopath.AbsoluteSystemPath {
	configHome := AbsoluteSystemPathFromUpstream(xdg.ConfigHome)
	return configHome.UntypedJoin("turborepo")
}
