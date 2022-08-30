// Package turbopath teaches the Go type system about six
// different types of paths:
// - AbsoluteSystemPath
// - RelativeSystemPath
// - AnchoredSystemPath
// - AbsoluteUnixPath
// - RelativeUnixPath
// - AnchoredUnixPath
//
// Between these two things it is assumed that we will be able to
// reasonably describe file paths being used within the system and
// have the type system enforce correctness instead of relying upon
// runtime code to accomplish the task.
//
// Absolute paths are, "absolute, including volume root." They are not
// portable between System and Unix.
//
// Relative paths are simply arbitrary path segments using a particular
// path delimiter. They are portable between System and Unix.
//
// Anchored paths are, "absolute, starting at a particular root."
// They are not aware of *what* their anchor is. It could be a repository,
// an `os.dirFS`, a package, `cwd`, or more. They are stored *without*
// a preceding delimiter for compatibility with `io/fs`. They are portable
// between System and Unix.
//
// In some future world everything in here can be optimized out at compile time.
// Everything is either `string` or `[]string`
//
// Much of this is dreadfully repetitive because of intentional
// limitations in the Go type system.
package turbopath

// AnchoredUnixPathArray is a type used to enable transform operations on arrays of paths.
type AnchoredUnixPathArray []AnchoredUnixPath

// RelativeSystemPathArray is a type used to enable transform operations on arrays of paths.
type RelativeSystemPathArray []RelativeSystemPath

// RelativeUnixPathArray is a type used to enable transform operations on arrays of paths.
type RelativeUnixPathArray []RelativeUnixPath

// ToStringArray enables ergonomic operations on arrays of RelativeSystemPath
func (source RelativeSystemPathArray) ToStringArray() []string {
	output := make([]string, len(source))
	for index, path := range source {
		output[index] = path.ToString()
	}
	return output
}

// ToStringArray enables ergonomic operations on arrays of RelativeUnixPath
func (source RelativeUnixPathArray) ToStringArray() []string {
	output := make([]string, len(source))
	for index, path := range source {
		output[index] = path.ToString()
	}
	return output
}

// ToSystemPathArray enables ergonomic operations on arrays of AnchoredUnixPath
func (source AnchoredUnixPathArray) ToSystemPathArray() []AnchoredSystemPath {
	output := make([]AnchoredSystemPath, len(source))
	for index, path := range source {
		output[index] = path.ToSystemPath()
	}
	return output
}

// The following methods exist to import a path string and cast it to the appropriate
// type. They exist to communicate intent and make it explicit that this is an
// intentional action, not a "helpful" insertion by the IDE.
//
// This is intended to map closely to the `unsafe` keyword, without the denotative
// meaning of `unsafe` in English. These are "trust me, I've checkex it" places, and
// intend to mark the places where we smuggle paths from outside the world of safe
// path handling into the world where we carefully consider the path to ensure safety.

// AbsoluteSystemPathFromUpstream takes a path string and casts it to an
// AbsoluteSystemPath without checking. If the input to this function is
// not an AbsoluteSystemPath it will result in downstream errors.
func AbsoluteSystemPathFromUpstream(path string) AbsoluteSystemPath {
	return AbsoluteSystemPath(path)
}

// AnchoredSystemPathFromUpstream takes a path string and casts it to an
// AnchoredSystemPath without checking. If the input to this function is
// not an AnchoredSystemPath it will result in downstream errors.
func AnchoredSystemPathFromUpstream(path string) AnchoredSystemPath {
	return AnchoredSystemPath(path)
}

// AnchoredUnixPathFromUpstream takes a path string and casts it to an
// AnchoredUnixPath without checking. If the input to this function is
// not an AnchoredUnixPath it will result in downstream errors.
func AnchoredUnixPathFromUpstream(path string) AnchoredUnixPath {
	return AnchoredUnixPath(path)
}

// RelativeSystemPathFromUpstream takes a path string and casts it to an
// RelativeSystemPath without checking. If the input to this function is
// not an RelativeSystemPath it will result in downstream errors.
func RelativeSystemPathFromUpstream(path string) RelativeSystemPath {
	return RelativeSystemPath(path)
}

// RelativeUnixPathFromUpstream takes a path string and casts it to an
// RelativeUnixPath without checking. If the input to this function is
// not an RelativeUnixPath it will result in downstream errors.
func RelativeUnixPathFromUpstream(path string) RelativeUnixPath {
	return RelativeUnixPath(path)
}
