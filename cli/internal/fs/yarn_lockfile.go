package fs

import (
	"fmt"

	"github.com/vercel/turborepo/cli/internal/lockfile"
)

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

var _ lockfile.Lockfile = (*YarnLockfile)(nil)

func (l *YarnLockfile) PossibleKeys(name string, version string) []string {
	return []string{
		fmt.Sprintf("%v@%v", name, version),
		fmt.Sprintf("%v@npm:%v", name, version),
	}
}

func (l *YarnLockfile) ResolvePackage(name string, version string) (string, string, bool) {
	lockfileKey1 := fmt.Sprintf("%v@%v", name, version)
	lockfileKey2 := fmt.Sprintf("%v@npm:%v", name, version)

	if e, ok := (*l)[lockfileKey1]; ok {
		return lockfileKey1, e.Version, true
	}
	if e, ok := (*l)[lockfileKey2]; ok {
		return lockfileKey2, e.Version, true
	}
	return "", "", false
}

func (l *YarnLockfile) AllDependencies(key string) (map[string]string, bool) {
	deps := map[string]string{}
	entry, ok := (*l)[key]
	if !ok {
		return deps, false
	}

	for name, version := range entry.Dependencies {
		deps[name] = version
	}
	for name, version := range entry.OptionalDependencies {
		deps[name] = version
	}

	return deps, true
}

func (l *YarnLockfile) SubLockfile(packages []string) (lockfile.Lockfile, error) {
	lockfile := make(YarnLockfile, len(packages))
	for _, key := range packages {
		entry, ok := (*l)[key]
		if ok {
			lockfile[key] = entry
		}
	}

	return &lockfile, nil
}
