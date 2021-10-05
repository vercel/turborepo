package api

// LanguageBackend is an abstraction across programming languages and thier related package managers

type LanguageBackend struct {
	// Name is the name of the language backend
	Name string

	// The filename of the specfile, e.g. "pyproject.toml" for
	// Poetry.
	//
	// This field is mandatory.
	Specfile string

	// The filename of the lockfile, e.g. "poetry.lock" for
	// Poetry.
	//
	// This field is mandatory.
	Lockfile string

	// List of filename globs that match against files written in
	// this programming language, e.g. "*.py" for Python. These
	// should not include any slashes, because they may be matched
	// in any subdirectory.
	//
	// This field is mandatory.
	FilenamePatterns []string

	// Return the path (relative to the project directory) in
	// which packages are installed. The path need not exist.
	GetPackageDir func() string

	// Return the list of workspace glob
	GetWorkspaceGlobs func() ([]string, error)
	// Returns run comman
	GetRunCommand func() []string
}
