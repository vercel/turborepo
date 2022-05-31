package turbopath

import (
	"path"
	"path/filepath"
)

// AbsoluteUnixPath is a root-relative path using Unix `/` separators.
type AbsoluteUnixPath string

// For interface reasons, we need a way to distinguish between
// Absolute/Repo/Relative/System/Unix/File paths so we stamp them.
func (AbsoluteUnixPath) absolutePathStamp() {}
func (AbsoluteUnixPath) unixPathStamp()     {}
func (AbsoluteUnixPath) filePathStamp()     {}

// ToSystemPath converts a AbsoluteUnixPath to a SystemPath.
func (p AbsoluteUnixPath) ToSystemPath() SystemPathInterface {
	return AbsoluteSystemPath(filepath.FromSlash(p.ToString()))
}

// ToUnixPath called on a AbsoluteUnixPath returns itself.
// It exists to enable simpler code at call sites.
func (p AbsoluteUnixPath) ToUnixPath() UnixPathInterface {
	return p
}

// Rel calculates the relative path between a AbsoluteUnixPath and any other UnixPath.
func (p AbsoluteUnixPath) Rel(basePath UnixPathInterface) (RelativeUnixPath, error) {
	processed, err := filepath.Rel(basePath.ToString(), p.ToString())
	return RelativeUnixPath(processed), err
}

// ToString returns a string represenation of this Path.
// Used for interfacing with APIs that require a string.
func (p AbsoluteUnixPath) ToString() string {
	return string(p)
}

// ToAbsoluteSystemPath converts from AbsoluteUnixPath to AbsoluteSystemPath.
func (p AbsoluteUnixPath) ToAbsoluteSystemPath() AbsoluteSystemPath {
	return p.ToSystemPath().(AbsoluteSystemPath)
}

// Join appends relative path segments to this AbsoluteUnixPath.
func (p AbsoluteUnixPath) Join(additional ...RelativeUnixPath) AbsoluteUnixPath {
	return AbsoluteUnixPath(path.Join(p.ToString(), path.Join(toStringArray(additional)...)))
}
