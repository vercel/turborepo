package turbopath

import "path/filepath"

// The goal is to be able to teach the Go type system about six
// different types of paths:
// - AbsoluteSystemPath
// - RelativeSystemPath
// - RepoRelativeSystemPath
// - AbsoluteUnixPath
// - RelativeUnixPath
// - RepoRelativeUnixPath
//
// There is a second portion which allows clustering particular
// dimensions of these four types:
// - AbsolutePathInterface
// - RelativePathInterface
// - RepoRelativePathInterface
// - UnixPathInterface
// - SystemPathInterface
// - FilePathInterface
//
// Between these two things it is assumed that we will be able to
// reasonably describe file paths being used within the system and
// have the type system enforce correctness instead of relying upon
// runtime code to accomplish the task.
//
// Much of this is dreadfully repetitive because of intentional
// limitations in the Go type system.

// AbsoluteUnixPathInterface specifies the members that mark an interface
// for structurally typing so the Go compiler can understand it.
type AbsoluteUnixPathInterface interface {
	filePathStamp()
	absolutePathStamp()
	unixPathStamp()

	ToString() string
}

// RelativeUnixPathInterface specifies the members that mark an interface
// for structurally typing so the Go compiler can understand it.
type RelativeUnixPathInterface interface {
	filePathStamp()
	relativePathStamp()
	unixPathStamp()

	ToString() string
}

// RepoRelativeUnixPathInterface specifies the members that mark an interface
// for structurally typing so the Go compiler can understand it.
type RepoRelativeUnixPathInterface interface {
	filePathStamp()
	relativePathStamp()
	repoRelativeStamp()
	unixPathStamp()

	ToString() string
}

// AbsoluteSystemPathInterface specifies the members that mark an interface
// for structurally typing so the Go compiler can understand it.
type AbsoluteSystemPathInterface interface {
	filePathStamp()
	absolutePathStamp()
	systemPathStamp()

	ToString() string
}

// RelativeSystemPathInterface specifies the members that mark an interface
// for structurally typing so the Go compiler can understand it.
type RelativeSystemPathInterface interface {
	filePathStamp()
	relativePathStamp()
	systemPathStamp()

	ToString() string
}

// RepoRelativeSystemPathInterface specifies the members that mark an interface
// for structurally typing so the Go compiler can understand it.
type RepoRelativeSystemPathInterface interface {
	filePathStamp()
	relativePathStamp()
	repoRelativeStamp()
	systemPathStamp()

	ToString() string
}

// AbsolutePathInterface specifies additional dimensions that a particular
// object can have that are independent from each other.
type AbsolutePathInterface interface {
	filePathStamp()
	absolutePathStamp()

	ToString() string
}

// RelativePathInterface specifies additional dimensions that a particular
// object can have that are independent from each other.
type RelativePathInterface interface {
	filePathStamp()
	relativePathStamp()

	ToString() string
}

// RepoRelativePathInterface specifies additional dimensions that a particular
// object can have that are independent from each other.
type RepoRelativePathInterface interface {
	filePathStamp()
	relativePathStamp()
	repoRelativeStamp()

	ToString() string
}

// UnixPathInterface specifies additional dimensions that a particular
// object can have that are independent from each other.
type UnixPathInterface interface {
	filePathStamp()
	unixPathStamp()

	Rel(UnixPathInterface) (RelativeUnixPath, error)
	ToSystemPath() SystemPathInterface
	ToUnixPath() UnixPathInterface
	ToString() string
}

// SystemPathInterface specifies additional dimensions that a particular
// object can have that are independent from each other.
type SystemPathInterface interface {
	filePathStamp()
	systemPathStamp()

	Rel(SystemPathInterface) (RelativeSystemPath, error)
	ToSystemPath() SystemPathInterface
	ToUnixPath() UnixPathInterface
	ToString() string
}

// FilePathInterface specifies additional dimensions that a particular
// object can have that are independent from each other.
type FilePathInterface interface {
	filePathStamp()

	ToSystemPath() SystemPathInterface
	ToUnixPath() UnixPathInterface
	ToString() string
}

// StringToUnixPath parses a path string from an unknown source
// and converts it into a Unix path. The only valid separator for
// a result from this call is `/`
func StringToUnixPath(path string) UnixPathInterface {
	if filepath.IsAbs(path) {
		return AbsoluteUnixPath(filepath.ToSlash(path))
	}
	return RelativeUnixPath(filepath.ToSlash(path))
}

// StringToSystemPath parses a path string from an unknown source
// and converts it into a system path. This means it could have
// any valid separators (`\` or `/`).
func StringToSystemPath(path string) SystemPathInterface {
	if filepath.IsAbs(path) {
		return AbsoluteSystemPath(filepath.FromSlash(path))
	}
	return RelativeSystemPath(filepath.FromSlash(path))
}
