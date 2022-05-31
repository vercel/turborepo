package turbopath

import "path/filepath"

// AbsoluteSystemPath is a root-relative path using system separators.
type AbsoluteSystemPath string

// For interface reasons, we need a way to distinguish between
// Absolute/Repo/Relative/System/Unix/File paths so we stamp them.
func (AbsoluteSystemPath) absolutePathStamp() {}
func (AbsoluteSystemPath) systemPathStamp()   {}
func (AbsoluteSystemPath) filePathStamp()     {}

// ToSystemPath called on a AbsoluteSystemPath returns itself.
// It exists to enable simpler code at call sites.
func (p AbsoluteSystemPath) ToSystemPath() SystemPathInterface {
	return p
}

// ToUnixPath converts a AbsoluteSystemPath to a UnixPath.
func (p AbsoluteSystemPath) ToUnixPath() UnixPathInterface {
	return AbsoluteUnixPath(filepath.ToSlash(p.ToString()))
}

// Rel calculates the relative path between a AbsoluteSystemPath and any other SystemPath.
func (p AbsoluteSystemPath) Rel(basePath SystemPathInterface) (RelativeSystemPath, error) {
	processed, err := filepath.Rel(basePath.ToString(), p.ToString())
	return RelativeSystemPath(processed), err
}

// ToString returns a string represenation of this Path.
// Used for interfacing with APIs that require a string.
func (p AbsoluteSystemPath) ToString() string {
	return string(p)
}

// ToAbsoluteUnixPath converts from AbsoluteSystemPath to AbsoluteUnixPath.
func (p AbsoluteSystemPath) ToAbsoluteUnixPath() AbsoluteUnixPath {
	return p.ToUnixPath().(AbsoluteUnixPath)
}

// Join appends relative path segments to this AbsoluteSystemPath.
func (p AbsoluteSystemPath) Join(additional ...RelativeSystemPath) AbsoluteSystemPath {
	return AbsoluteSystemPath(filepath.Join(p.ToString(), filepath.Join(toStringArray(additional)...)))
}
