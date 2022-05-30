package turbopath

import "path/filepath"

// RelativeSystemPath is a relative path using system separators.
type RelativeSystemPath string

// For interface reasons, we need a way to distinguish between
// Absolute/Repo/Relative/System/Unix/File paths so we stamp them.
func (RelativeSystemPath) relativePathStamp() {}
func (RelativeSystemPath) systemPathStamp()   {}
func (RelativeSystemPath) filePathStamp()     {}

// ToSystemPath called on a RelativeSystemPath returns itself.
// It exists to enable simpler code at call sites.
func (p RelativeSystemPath) ToSystemPath() SystemPathInterface {
	return p
}

// ToUnixPath converts a RelativeSystemPath to a UnixPath.
func (p RelativeSystemPath) ToUnixPath() UnixPathInterface {
	return RelativeUnixPath(filepath.ToSlash(p.ToString()))
}

// Rel calculates the relative path between a RelativeSystemPath and any other SystemPath.
func (p RelativeSystemPath) Rel(basePath SystemPathInterface) (RelativeSystemPath, error) {
	processed, err := filepath.Rel(basePath.ToString(), p.ToString())
	return RelativeSystemPath(processed), err
}

// ToString returns a string represenation of this Path.
// Used for interfacing with APIs that require a string.
func (p RelativeSystemPath) ToString() string {
	return string(p)
}

// ToRelativeUnixPath converts from RelativeSystemPath to RelativeUnixPath.
func (p RelativeSystemPath) ToRelativeUnixPath() RelativeUnixPath {
	return p.ToUnixPath().(RelativeUnixPath)
}

// Join appends relative path segments to this RelativeSystemPath.
func (p RelativeSystemPath) Join(additional RelativeSystemPath) RelativeSystemPath {
	return RelativeSystemPath(filepath.Join(p.ToString(), additional.ToString()))
}
