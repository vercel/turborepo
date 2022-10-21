package lockfile

import (
	"encoding/json"
	"fmt"
	"io"
	"strings"

	"github.com/vercel/turborepo/cli/internal/turbopath"
)

// NpmLockfile representation of package-lock.json
type NpmLockfile struct {
	Name            string `json:"name"`
	Version         string `json:"version"`
	LockfileVersion int    `json:"lockfileVersion,omitempty"`
	Requires        bool   `json:"requires,omitempty"`
	// Keys are paths to package.json, can be nested in node_modules
	Packages map[string]NpmPackage `json:"packages,omitempty"`
	// Legacy info for npm version 5 & 6
	Dependencies map[string]NpmDependency `json:"dependencies,omitempty"`
}

// NpmPackage Representation of dependencies used in LockfileVersion 2+
type NpmPackage struct {
	// Only used for root level package
	Name string `json:"name,omitempty"`

	Version   string `json:"version,omitempty"`
	Resolved  string `json:"resolved,omitempty"`
	Integrity string `json:"integrity,omitempty"`
	Link      bool   `json:"link,omitempty"`

	Dev      bool `json:"dev,omitempty"`
	Optional bool `json:"optional,omitempty"`
	// Only used if included as a dev dep of one package and an optional dep of another
	// An optional dep of a dev dep will have dev and optional set to true
	DevOptional      bool `json:"devOptional,omitempty"`
	InBundle         bool `json:"inBundle,omitempty"`
	HasInstallScript bool `json:"hasInstallScript,omitempty"`
	HasShrinkwrap    bool `json:"hasShrinkwrap,omitempty"`
	Extraneous       bool `json:"extraneous,omitempty"`

	Dependencies         map[string]string `json:"dependencies,omitempty"`
	DevDependencies      map[string]string `json:"devDependencies,omitempty"`
	PeerDependencies     map[string]string `json:"peerDependencies,omitempty"`
	OptionalDependencies map[string]string `json:"optionalDependencies,omitempty"`

	Bin map[string]string `json:"bin,omitempty"`

	// Engines has two valid formats historically, an array and an object.
	// Since we don't use this property we just need to propagate it through.
	//
	// Go serializes correctly even with `interface{}`.
	Engines interface{} `json:"engines,omitempty"`

	CPU []string `json:"cpu,omitempty"`
	OS  []string `json:"os,omitempty"`

	// Only used for root level package
	Workspaces []string `json:"workspaces,omitempty"`
}

// NpmDependency Legacy representation of dependencies
type NpmDependency struct {
	Version      string                   `json:"version"`
	Resolved     string                   `json:"resolved,omitempty"`
	Integrity    string                   `json:"integrity,omitempty"`
	Bundled      bool                     `json:"bundled,omitempty"`
	Dev          bool                     `json:"dev,omitempty"`
	Optional     bool                     `json:"optional,omitempty"`
	Requires     map[string]string        `json:"requires,omitempty"`
	Dependencies map[string]NpmDependency `json:"dependencies,omitempty"`
}

var _ Lockfile = (*NpmLockfile)(nil)

// ResolvePackage Given a workspace, a package it imports and version returns the key, resolved version, and if it was found
func (l *NpmLockfile) ResolvePackage(workspacePath turbopath.AnchoredUnixPath, name string, version string) (Package, error) {
	_, ok := l.Packages[workspacePath.ToString()]
	if !ok {
		return Package{}, fmt.Errorf("No package found in lockfile for '%s'", workspacePath)
	}

	// AllDependencies will return a key to avoid choosing the incorrect transitive dep
	if entry, ok := l.Packages[name]; ok {
		return Package{
			Key:     name,
			Version: entry.Version,
			Found:   true,
		}, nil
	}

	// If we didn't find the entry just using name, then this is an initial call to ResolvePackage
	// based on information coming from internal packages' package.json
	// First we check if the workspace uses a nested version of the package
	nestedPath := fmt.Sprintf("%s/node_modules/%s", workspacePath, name)
	if entry, ok := l.Packages[nestedPath]; ok {
		return Package{
			Key:     nestedPath,
			Version: entry.Version,
			Found:   true,
		}, nil
	}

	// Next we check for a top level version of the package
	hoistedPath := fmt.Sprintf("node_modules/%s", name)
	if entry, ok := l.Packages[hoistedPath]; ok {
		return Package{
			Key:     hoistedPath,
			Version: entry.Version,
			Found:   true,
		}, nil
	}

	return Package{}, nil
}

// AllDependencies Given a lockfile key return all (dev/optional/peer) dependencies of that package
func (l *NpmLockfile) AllDependencies(key string) (map[string]string, bool) {
	entry, ok := l.Packages[key]
	if !ok {
		return nil, false
	}
	deps := make(map[string]string, len(entry.Dependencies)+len(entry.DevDependencies)+len(entry.PeerDependencies)+len(entry.OptionalDependencies))
	addDep := func(d map[string]string) {
		for name := range d {
			for _, possibleKey := range possibleNpmDeps(key, name) {
				if entry, ok := l.Packages[possibleKey]; ok {
					deps[possibleKey] = entry.Version
					break
				}
			}
		}
	}

	addDep(entry.Dependencies)
	addDep(entry.DevDependencies)
	addDep(entry.OptionalDependencies)
	addDep(entry.PeerDependencies)

	return deps, true
}

// Subgraph Given a list of lockfile keys returns a Lockfile based off the original one that only contains the packages given
func (l *NpmLockfile) Subgraph(workspacePackages []turbopath.AnchoredSystemPath, packages []string) (Lockfile, error) {
	prunedPackages := make(map[string]NpmPackage, len(packages))
	for _, pkg := range packages {
		if entry, ok := l.Packages[pkg]; ok {
			prunedPackages[pkg] = entry
		} else {
			return nil, fmt.Errorf("No lockfile entry found for %s", pkg)
		}
	}
	if rootEntry, ok := l.Packages[""]; ok {
		prunedPackages[""] = rootEntry
	}
	for _, workspacePackages := range workspacePackages {
		workspacePkg := workspacePackages.ToUnixPath().ToString()
		if workspaceEntry, ok := l.Packages[workspacePkg]; ok {
			prunedPackages[workspacePkg] = workspaceEntry
		} else {
			return nil, fmt.Errorf("No lockfile entry found for %s", workspacePkg)
		}
		// Each workspace package has a fake version of the package that links back to the original
		// but is needed for dependency resolution.
		for key, entry := range l.Packages {
			if entry.Resolved == workspacePkg {
				prunedPackages[key] = entry
				break
			}
		}
	}

	return &NpmLockfile{
		Name:    l.Name,
		Version: l.Version,
		// Forcible upgrade to version 3 since we aren't including backward compatibility capabilities
		LockfileVersion: 3,
		Requires:        l.Requires,
		Packages:        prunedPackages,
		// We omit dependencies since they are only used for supporting npm <=6
	}, nil
}

// Encode the lockfile representation and write it to the given writer
func (l *NpmLockfile) Encode(w io.Writer) error {
	encoder := json.NewEncoder(w)
	encoder.SetIndent("", "  ")
	encoder.SetEscapeHTML(false)
	return encoder.Encode(l)
}

// Patches return a list of patches used in the lockfile
func (l *NpmLockfile) Patches() []turbopath.AnchoredUnixPath {
	return nil
}

// DecodeNpmLockfile Parse contents of package-lock.json into NpmLockfile
func DecodeNpmLockfile(content []byte) (*NpmLockfile, error) {
	var lockfile NpmLockfile
	if err := json.Unmarshal(content, &lockfile); err != nil {
		return nil, err
	}

	// LockfileVersion <=1 is for npm <=6
	ancientLockfile := lockfile.LockfileVersion <= 1 || (len(lockfile.Dependencies) > 0 && len(lockfile.Packages) == 0)

	if ancientLockfile {
		// TODO: Older versions of package-lock.json required crawling through node_modules and other
		// various fixups to make it a deterministic lockfile.
		// See https://github.com/npm/cli/blob/9609e9eed87c735f0319ac0af265f4d406cbf800/workspaces/arborist/lib/shrinkwrap.js#L674
		return nil, fmt.Errorf("Support for lockfiles without a 'packages' field isn't implemented yet")
	}

	return &lockfile, nil
}

// returns a list of possible keys for a dependency of package key
func possibleNpmDeps(key string, dep string) []string {
	possibleDeps := []string{fmt.Sprintf("%s/node_modules/%s", key, dep)}

	curr := key
	for curr != "" {
		next := npmPathParent(curr)
		possibleDeps = append(possibleDeps, fmt.Sprintf("%snode_modules/%s", next, dep))
		curr = next
	}

	return possibleDeps
}

func npmPathParent(key string) string {
	if index := strings.LastIndex(key, "node_modules/"); index != -1 {
		return key[0:index]
	}
	return ""
}
