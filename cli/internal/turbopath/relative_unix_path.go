package turbopath

import (
	"path"
	"path/filepath"
)

// RelativeUnixPath is a relative path using Unix `/` separators.
type RelativeUnixPath string

// For interface reasons, we need a way to distinguish between
// Absolute/Anchored/Relative/System/Unix/File paths so we stamp them.
func (RelativeUnixPath) relativePathStamp() {}
func (RelativeUnixPath) unixPathStamp()     {}
func (RelativeUnixPath) filePathStamp()     {}

// ToString returns a string represenation of this Path.
// Used for interfacing with APIs that require a string.
func (p RelativeUnixPath) ToString() string {
	return string(p)
}

// ToSystemPath converts a RelativeUnixPath to a RelativeSystemPath.
func (p RelativeUnixPath) ToSystemPath() RelativeSystemPath {
	return RelativeSystemPath(filepath.FromSlash(p.ToString()))
}

// ToUnixPath converts a RelativeUnixPath to a RelativeSystemPath.
func (p RelativeUnixPath) ToUnixPath() RelativeUnixPath {
	return p
}

// Join appends relative path segments to this RelativeUnixPath.
func (p RelativeUnixPath) Join(additional ...RelativeUnixPath) RelativeUnixPath {
	cast := RelativeUnixPathArray(additional)
	return RelativeUnixPath(path.Join(p.ToString(), path.Join(cast.ToStringArray()...)))
}
