package fs

type LockfileEntry struct {
	// resolved version for the particular entry based on the provided semver revision
	Version   string `yaml:"version"`
	Resolved  string `yaml:"resolved"`
	Integrity string `yaml:"integrity"`
	// the list of unresolved modules and revisions (e.g. type-detect : ^4.0.0)
	Dependencies map[string]string `yaml:"dependencies,omitempty"`
	// the list of unresolved modules and revisions (e.g. type-detect : ^4.0.0)
	OptionalDependencies map[string]string `yaml:"optionalDependencies,omitempty"`
}

type YarnLockfile map[string]*LockfileEntry
