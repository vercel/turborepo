package turbopath

import (
	"path"
	"path/filepath"
)

// AbsoluteUnixPath is a root-relative path using Unix `/` separators.
type AbsoluteUnixPath string

// ToString returns a string represenation of this Path.
// Used for interfacing with APIs that require a string.
func (p AbsoluteUnixPath) ToString() string {
	return string(p)
}

// RelativeTo calculates the relative path between two `AbsoluteUnixPath`s.
func (p AbsoluteUnixPath) RelativeTo(basePath AbsoluteUnixPath) (AnchoredUnixPath, error) {
	processed, err := filepath.Rel(basePath.ToString(), p.ToString())
	return AnchoredUnixPath(processed), err
}

// Join appends relative path segments to this AbsoluteUnixPath.
func (p AbsoluteUnixPath) Join(additional ...RelativeUnixPath) AbsoluteUnixPath {
	cast := RelativeUnixPathArray(additional)
	return AbsoluteUnixPath(path.Join(p.ToString(), path.Join(cast.ToStringArray()...)))
}
