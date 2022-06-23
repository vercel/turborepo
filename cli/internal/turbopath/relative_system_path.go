package turbopath

import "path/filepath"

// RelativeSystemPath is a relative path using system separators.
type RelativeSystemPath string

// ToString returns a string represenation of this Path.
// Used for interfacing with APIs that require a string.
func (p RelativeSystemPath) ToString() string {
	return string(p)
}

// ToSystemPath returns itself.
func (p RelativeSystemPath) ToSystemPath() RelativeSystemPath {
	return p
}

// ToUnixPath converts from RelativeSystemPath to RelativeUnixPath.
func (p RelativeSystemPath) ToUnixPath() RelativeUnixPath {
	return RelativeUnixPath(filepath.ToSlash(p.ToString()))
}

// Join appends relative path segments to this RelativeSystemPath.
func (p RelativeSystemPath) Join(additional ...RelativeSystemPath) RelativeSystemPath {
	cast := RelativeSystemPathArray(additional)
	return RelativeSystemPath(filepath.Join(p.ToString(), filepath.Join(cast.ToStringArray()...)))
}
