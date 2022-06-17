// Package turbopath teaches the Go type system about six
// different types of paths:
// - AbsoluteSystemPath
// - RelativeSystemPath
// - AnchoredSystemPath
// - AbsoluteUnixPath
// - RelativeUnixPath
// - AnchoredUnixPath
//
// There is a second portion which allows clustering particular
// dimensions of these four types:
// - AbsolutePathInterface
// - RelativePathInterface
// - AnchoredPathInterface
// - UnixPathInterface
// - SystemPathInterface
// - FilePathInterface
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

	ToSystemPath() RelativeSystemPath
	ToUnixPath() RelativeUnixPath
	ToString() string
}

// AnchoredPathInterface specifies additional dimensions that a particular
// object can have that are independent from each other.
type AnchoredPathInterface interface {
	filePathStamp()
	anchoredStamp()

	ToSystemPath() AnchoredSystemPath
	ToUnixPath() AnchoredUnixPath
	ToString() string
}

// UnixPathInterface specifies additional dimensions that a particular
// object can have that are independent from each other.
type UnixPathInterface interface {
	filePathStamp()
	unixPathStamp()

	ToString() string
}

// SystemPathInterface specifies additional dimensions that a particular
// object can have that are independent from each other.
type SystemPathInterface interface {
	filePathStamp()
	systemPathStamp()

	ToString() string
}

// FilePathInterface specifies additional dimensions that a particular
// object can have that are independent from each other.
type FilePathInterface interface {
	filePathStamp()

	ToString() string
}

// AbsoluteSystemPathArray is a type used to enable transform operations on arrays of paths.
type AbsoluteSystemPathArray []AbsoluteSystemPath

// AbsoluteUnixPathArray is a type used to enable transform operations on arrays of paths.
type AbsoluteUnixPathArray []AbsoluteUnixPath

// AnchoredSystemPathArray is a type used to enable transform operations on arrays of paths.
type AnchoredSystemPathArray []AnchoredSystemPath

// AnchoredUnixPathArray is a type used to enable transform operations on arrays of paths.
type AnchoredUnixPathArray []AnchoredUnixPath

// RelativeSystemPathArray is a type used to enable transform operations on arrays of paths.
type RelativeSystemPathArray []RelativeSystemPath

// RelativeUnixPathArray is a type used to enable transform operations on arrays of paths.
type RelativeUnixPathArray []RelativeUnixPath

// ToStringArray enables ergonomic operations on arrays of AbsoluteSystemPath
func (source AbsoluteSystemPathArray) ToStringArray() []string {
	output := make([]string, len(source))
	for index, path := range source {
		output[index] = path.ToString()
	}
	return output
}

// ToStringArray enables ergonomic operations on arrays of AbsoluteUnixPath
func (source AbsoluteUnixPathArray) ToStringArray() []string {
	output := make([]string, len(source))
	for index, path := range source {
		output[index] = path.ToString()
	}
	return output
}

// ToStringArray enables ergonomic operations on arrays of AnchoredSystemPath
func (source AnchoredSystemPathArray) ToStringArray() []string {
	output := make([]string, len(source))
	for index, path := range source {
		output[index] = path.ToString()
	}
	return output
}

// ToStringArray enables ergonomic operations on arrays of AnchoredUnixPath
func (source AnchoredUnixPathArray) ToStringArray() []string {
	output := make([]string, len(source))
	for index, path := range source {
		output[index] = path.ToString()
	}
	return output
}

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

// ToUnixPathArray enables ergonomic operations on arrays of AnchoredSystemPath
func (source AnchoredSystemPathArray) ToUnixPathArray() []AnchoredUnixPath {
	output := make([]AnchoredUnixPath, len(source))
	for index, path := range source {
		output[index] = path.ToUnixPath()
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

// ToUnixPathArray enables ergonomic operations on arrays of RelativeSystemPath
func (source RelativeSystemPathArray) ToUnixPathArray() []RelativeUnixPath {
	output := make([]RelativeUnixPath, len(source))
	for index, path := range source {
		output[index] = path.ToUnixPath()
	}
	return output
}

// ToSystemPathArray enables ergonomic operations on arrays of RelativeUnixPath
func (source RelativeUnixPathArray) ToSystemPathArray() []RelativeSystemPath {
	output := make([]RelativeSystemPath, len(source))
	for index, path := range source {
		output[index] = path.ToSystemPath()
	}
	return output
}
