package turbopath

import (
	"path"
	"path/filepath"
)

// RepoRelativeUnixPath is a relative path from the repository using Unix `/` separators.
type RepoRelativeUnixPath string

// For interface reasons, we need a way to distinguish between
// Absolute/Repo/Relative/System/Unix/File paths so we stamp them.
func (RepoRelativeUnixPath) relativePathStamp() {}
func (RepoRelativeUnixPath) unixPathStamp()     {}
func (RepoRelativeUnixPath) filePathStamp()     {}

// ToSystemPath converts a RelativeUnixPath to a SystemPath.
func (p RepoRelativeUnixPath) ToSystemPath() SystemPathInterface {
	return RepoRelativeSystemPath(filepath.FromSlash(p.ToString()))
}

// ToUnixPath called on a RepoRelativeUnixPath returns itself.
// It exists to enable simpler code at call sites.
func (p RepoRelativeUnixPath) ToUnixPath() UnixPathInterface {
	return p
}

// Rel calculates the relative path between a RepoRelativeUnixPath and any other UnixPath.
func (p RepoRelativeUnixPath) Rel(basePath UnixPathInterface) (RelativeUnixPath, error) {
	processed, err := filepath.Rel(basePath.ToString(), p.ToString())
	return RelativeUnixPath(processed), err
}

// ToString returns a string represenation of this Path.
// Used for interfacing with APIs that require a string.
func (p RepoRelativeUnixPath) ToString() string {
	return string(p)
}

// ToRepoRelativeSystemPath converts from RelativeUnixPath to RelativeSystemPath.
func (p RepoRelativeUnixPath) ToRepoRelativeSystemPath() RepoRelativeSystemPath {
	return p.ToSystemPath().(RepoRelativeSystemPath)
}

// ToRelativeUnixPath converts from RepoRelativeUnixPath to RelativeSystemPath.
func (p RepoRelativeUnixPath) ToRelativeUnixPath() RelativeUnixPath {
	return RelativeUnixPath(p)
}

// Join appends relative path segments to this RelativeUnixPath.
func (p RepoRelativeUnixPath) Join(additional RelativeUnixPath) RepoRelativeUnixPath {
	return RepoRelativeUnixPath(path.Join(p.ToString(), additional.ToString()))
}
