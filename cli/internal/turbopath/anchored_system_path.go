package turbopath

import "path/filepath"

// AnchoredSystemPath is a path stemming from a specified root using system separators.
type AnchoredSystemPath string

// ToString returns a string represenation of this Path.
// Used for interfacing with APIs that require a string.
func (p AnchoredSystemPath) ToString() string {
	return string(p)
}

// ToStringDuringMigration returns the string representation of this path, and is for
// use in situations where we expect a future path migration to remove the need for the
// string representation
func (p AnchoredSystemPath) ToStringDuringMigration() string {
	return string(p)
}

// ToSystemPath returns itself.
func (p AnchoredSystemPath) ToSystemPath() AnchoredSystemPath {
	return p
}

// ToUnixPath converts a AnchoredSystemPath to a AnchoredUnixPath.
func (p AnchoredSystemPath) ToUnixPath() AnchoredUnixPath {
	return AnchoredUnixPath(filepath.ToSlash(p.ToString()))
}

// RelativeTo calculates the relative path between two AnchoredSystemPath`s.
func (p AnchoredSystemPath) RelativeTo(basePath AnchoredSystemPath) (AnchoredSystemPath, error) {
	processed, err := filepath.Rel(basePath.ToString(), p.ToString())
	return AnchoredSystemPath(processed), err
}

// RestoreAnchor prefixes the AnchoredSystemPath with its anchor to return an AbsoluteSystemPath.
func (p AnchoredSystemPath) RestoreAnchor(anchor AbsoluteSystemPath) AbsoluteSystemPath {
	return AbsoluteSystemPath(filepath.Join(anchor.ToString(), p.ToString()))
}

// Join appends relative path segments to this AnchoredSystemPath.
func (p AnchoredSystemPath) Join(additional ...RelativeSystemPath) AnchoredSystemPath {
	cast := RelativeSystemPathArray(additional)
	return AnchoredSystemPath(filepath.Join(p.ToString(), filepath.Join(cast.ToStringArray()...)))
}
