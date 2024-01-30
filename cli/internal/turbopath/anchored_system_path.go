package turbopath

import (
	"os"
	"path/filepath"
	"strings"
)

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

// Dir returns filepath.Dir for the path.
func (p AnchoredSystemPath) Dir() AnchoredSystemPath {
	return AnchoredSystemPath(filepath.Dir(p.ToString()))
}

// Join appends relative path segments to this AnchoredSystemPath.
func (p AnchoredSystemPath) Join(additional ...RelativeSystemPath) AnchoredSystemPath {
	cast := RelativeSystemPathArray(additional)
	return AnchoredSystemPath(filepath.Join(p.ToString(), filepath.Join(cast.ToStringArray()...)))
}

// HasPrefix is strings.HasPrefix for paths, ensuring that it matches on separator boundaries.
// This does NOT perform Clean in advance.
func (p AnchoredSystemPath) HasPrefix(prefix AnchoredSystemPath) bool {
	prefixLen := len(prefix)
	pathLen := len(p)

	if prefixLen > pathLen {
		// Can't be a prefix if longer.
		return false
	} else if prefixLen == pathLen {
		// Can be a prefix if they're equal, but otherwise no.
		return p == prefix
	}

	// otherPath is definitely shorter than p.
	// We need to confirm that p[len(otherPath)] is a system separator.

	return strings.HasPrefix(p.ToString(), prefix.ToString()) && os.IsPathSeparator(p[prefixLen])
}
