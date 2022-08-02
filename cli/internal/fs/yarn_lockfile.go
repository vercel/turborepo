package fs

type LockfileEntry struct {
	// resolved version for the particular entry based on the provided semver revision
	Version    string `yaml:"version"`
	Resolved   string `yaml:"resolved,omitempty"`
	Integrity  string `yaml:"integrity,omitempty"`
	Resolution string `yaml:"resolution,omitempty"`
	// the list of unresolved modules and revisions (e.g. type-detect : ^4.0.0)
	Dependencies map[string]string `yaml:"dependencies,omitempty"`
	// the list of unresolved modules and revisions (e.g. type-detect : ^4.0.0)
	Bin                  map[string]string          `yaml:"bin,omitempty"`
	OptionalDependencies map[string]string          `yaml:"optionalDependencies,omitempty"`
	Checksum             string                     `yaml:"checksum,omitempty"`
	Conditions           string                     `yaml:"conditions,omitempty"`
	LanguageName         string                     `yaml:"languageName,omitempty"`
	LinkType             string                     `yaml:"linkType,omitempty"`
	PeerDependencies     map[string]string          `yaml:"peerDependencies,omitempty"`
	PeerDependenciesMeta map[string]map[string]bool `yaml:"peerDependenciesMeta,omitempty"`
}

type YarnLockfile map[string]*LockfileEntry
