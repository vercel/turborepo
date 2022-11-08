package turbopath

import (
	"path"
	"path/filepath"
)

// AnchoredUnixPath is a path stemming from a specified root using Unix `/` separators.
type AnchoredUnixPath string

// ToString returns a string represenation of this Path.
// Used for interfacing with APIs that require a string.
func (p AnchoredUnixPath) ToString() string {
	return string(p)
}

// ToSystemPath converts a AnchoredUnixPath to a AnchoredSystemPath.
func (p AnchoredUnixPath) ToSystemPath() AnchoredSystemPath {
	return AnchoredSystemPath(filepath.FromSlash(p.ToString()))
}

// ToUnixPath returns itself.
func (p AnchoredUnixPath) ToUnixPath() AnchoredUnixPath {
	return p
}

// Join appends relative path segments to this RelativeUnixPath.
func (p AnchoredUnixPath) Join(additional ...RelativeUnixPath) AnchoredUnixPath {
	cast := RelativeUnixPathArray(additional)
	return AnchoredUnixPath(path.Join(p.ToString(), path.Join(cast.ToStringArray()...)))
}
