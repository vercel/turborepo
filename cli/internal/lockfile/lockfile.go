package lockfile

import "io"

// Lockfile Interface for general operations that work accross all lockfiles
type Lockfile interface {
	// PossibleKeys Given a package name and version return all of the keys it might appear as in the lockfile
	PossibleKeys(name string, version string) []string
	// ResovlePackage Given a package and version returns the key, resolved version, and if it was found
	ResolvePackage(name string, version string) (string, string, bool)
	// AllDependencies Given a lockfile key return all (dev/optional/peer) dependencies of that package
	AllDependencies(key string) (map[string]string, bool)
	// SubLockfile Given a list of lockfile keys returns a Lockfile based off the original one that only contains the packages given
	SubLockfile(packages []string) (Lockfile, error)
	// Encode encode the lockfile representation and write it to the given writer
	Encode(w io.Writer) error
}
