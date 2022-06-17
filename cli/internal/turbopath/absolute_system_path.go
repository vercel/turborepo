package turbopath

import "path/filepath"

// AbsoluteSystemPath is a root-relative path using system separators.
type AbsoluteSystemPath string

// ToString returns a string represenation of this Path.
// Used for interfacing with APIs that require a string.
func (p AbsoluteSystemPath) ToString() string {
	return string(p)
}

// RelativeTo calculates the relative path between two `AbsoluteSystemPath`s.
func (p AbsoluteSystemPath) RelativeTo(basePath AbsoluteSystemPath) (AnchoredSystemPath, error) {
	processed, err := filepath.Rel(basePath.ToString(), p.ToString())
	return AnchoredSystemPath(processed), err
}

// Join appends relative path segments to this AbsoluteSystemPath.
func (p AbsoluteSystemPath) Join(additional ...RelativeSystemPath) AbsoluteSystemPath {
	cast := RelativeSystemPathArray(additional)
	return AbsoluteSystemPath(filepath.Join(p.ToString(), filepath.Join(cast.ToStringArray()...)))
}
