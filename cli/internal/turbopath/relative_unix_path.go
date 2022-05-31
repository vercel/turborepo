package turbopath

import (
	"path"
	"path/filepath"
)

// RelativeUnixPath is a relative path using Unix `/` separators.
type RelativeUnixPath string

// For interface reasons, we need a way to distinguish between
// Absolute/Repo/Relative/System/Unix/File paths so we stamp them.
func (RelativeUnixPath) relativePathStamp() {}
func (RelativeUnixPath) unixPathStamp()     {}
func (RelativeUnixPath) filePathStamp()     {}

// ToSystemPath converts a RelativeUnixPath to a SystemPath.
func (p RelativeUnixPath) ToSystemPath() SystemPathInterface {
	return RelativeSystemPath(filepath.FromSlash(p.ToString()))
}

// ToUnixPath called on a RelativeUnixPath returns itself.
// It exists to enable simpler code at call sites.
func (p RelativeUnixPath) ToUnixPath() UnixPathInterface {
	return p
}

// Rel calculates the relative path between a RelativeUnixPath and any other UnixPath.
func (p RelativeUnixPath) Rel(basePath UnixPathInterface) (RelativeUnixPath, error) {
	processed, err := filepath.Rel(basePath.ToString(), p.ToString())
	return RelativeUnixPath(processed), err
}

// ToString returns a string represenation of this Path.
// Used for interfacing with APIs that require a string.
func (p RelativeUnixPath) ToString() string {
	return string(p)
}

// ToRelativeSystemPath converts from RelativeUnixPath to RelativeSystemPath.
func (p RelativeUnixPath) ToRelativeSystemPath() RelativeSystemPath {
	return p.ToSystemPath().(RelativeSystemPath)
}

// Join appends relative path segments to this RelativeUnixPath.
func (p RelativeUnixPath) Join(additional ...RelativeUnixPath) RelativeUnixPath {
	return RelativeUnixPath(path.Join(p.ToString(), path.Join(toStringArray(additional)...)))
}
