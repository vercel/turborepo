package turbopath

import "path/filepath"

// RepoRelativeSystemPath is a relative path from the repository using system separators.
type RepoRelativeSystemPath string

// For interface reasons, we need a way to distinguish between
// Absolute/Repo/Relative/System/Unix/File paths so we stamp them.
func (RepoRelativeSystemPath) relativePathStamp() {}
func (RepoRelativeSystemPath) systemPathStamp()   {}
func (RepoRelativeSystemPath) filePathStamp()     {}

// ToSystemPath called on a RelativeSystemPath returns itself.
// It exists to enable simpler code at call sites.
func (p RepoRelativeSystemPath) ToSystemPath() SystemPathInterface {
	return p
}

// ToUnixPath converts a RepoRelativeSystemPath to a UnixPath.
func (p RepoRelativeSystemPath) ToUnixPath() UnixPathInterface {
	return RepoRelativeUnixPath(filepath.ToSlash(p.ToString()))
}

// Rel calculates the relative path between a RelativeSystemPath and any other SystemPath.
func (p RepoRelativeSystemPath) Rel(basePath SystemPathInterface) (RelativeSystemPath, error) {
	processed, err := filepath.Rel(basePath.ToString(), p.ToString())
	return RelativeSystemPath(processed), err
}

// ToString returns a string represenation of this Path.
// Used for interfacing with APIs that require a string.
func (p RepoRelativeSystemPath) ToString() string {
	return string(p)
}

// ToRepoRelativeUnixPath converts from RelativeSystemPath to RepoRelativeUnixPath.
func (p RepoRelativeSystemPath) ToRepoRelativeUnixPath() RepoRelativeUnixPath {
	return p.ToUnixPath().(RepoRelativeUnixPath)
}

// ToRelativeSystemPath converts from RepoRelativeSystemPath to RelativeSystemPath.
func (p RepoRelativeSystemPath) ToRelativeSystemPath() RelativeSystemPath {
	return RelativeSystemPath(p)
}

// Join appends relative path segments to this RepoRelativeSystemPath.
func (p RepoRelativeSystemPath) Join(additional ...RelativeSystemPath) RepoRelativeSystemPath {
	return RepoRelativeSystemPath(filepath.Join(p.ToString(), filepath.Join(toStringArray(additional)...)))
}
