package lockfile

import (
	"fmt"
	"io"

	"github.com/pkg/errors"
	"gopkg.in/yaml.v3"
)

// PnpmLockfile Go representation of the contents of 'pnpm-lock.yaml'
// Reference https://github.com/pnpm/pnpm/blob/main/packages/lockfile-types/src/index.ts
type PnpmLockfile struct {
	Version            float32                    `yaml:"lockfileVersion"`
	Importers          map[string]ProjectSnapshot `yaml:"importers"`
	Packages           map[string]PackageSnapshot `yaml:"packages,omitempty"`
	NeverBuiltDeps     []string                   `yaml:"neverBuiltDependencies,omitempty"`
	OnlyBuiltDeps      []string                   `yaml:"onlyBuiltDependencies,omitempty"`
	Overrides          map[string]string          `yaml:"overrides,omitempty"`
	PackageExtChecksum string                     `yaml:"packageExtensionsChecksum,omitempty"`
	PatchedDeps        map[string]PatchFile       `yaml:"patchedDependencies,omitempty"`
}

var _ Lockfile = (*PnpmLockfile)(nil)

// ProjectSnapshot Snapshot used to represent projects in the importers section
type ProjectSnapshot struct {
	Specifiers           map[string]string         `yaml:"specifiers"`
	Dependencies         map[string]string         `yaml:"dependencies,omitempty"`
	OptionalDependencies map[string]string         `yaml:"optionalDependencies,omitempty"`
	DevDependencies      map[string]string         `yaml:"devDependencies,omitempty"`
	DependenciesMeta     map[string]DependencyMeta `yaml:"dependenciesMeta,omitempty"`
	PublishDirectory     string                    `yaml:"publishDirectory,omitempty"`
}

// PackageSnapshot Snapshot used to represent a package in the packages setion
type PackageSnapshot struct {
	ID string `yaml:"id,omitempty"`

	Resolution PackageResolution `yaml:"resolution,flow"`
	Engines    struct {
		Node string `yaml:"node"`
		NPM  string `yaml:"npm,omitempty"`
	} `yaml:"engines,omitempty,flow"`
	CPU           []string `yaml:"cpu,omitempty,flow"`
	Os            []string `yaml:"os,omitempty,flow"`
	HasBin        bool     `yaml:"hasBin,omitempty"`
	RequiresBuild bool     `yaml:"requiresBuild,omitempty"`

	PeerDependencies     map[string]string `yaml:"peerDependencies,omitempty"`
	PeerDependenciesMeta map[string]struct {
		Optional bool `yaml:"optional"`
	} `yaml:"peerDependenciesMeta,omitempty"`
	Dependencies         map[string]string `yaml:"dependencies,omitempty"`
	OptionalDependencies map[string]string `yaml:"optionalDependencies,omitempty"`
	TransitivePeerDeps   []string          `yaml:"transitivePeerDependencies,omitempty"`
	BundledDependencies  []string          `yaml:"bundledDependencies,omitempty"`

	Dev      bool `yaml:"dev"`
	Optional bool `yaml:"optional,omitempty"`
	Patched  bool `yaml:"patched,omitempty"`
	Prepare  bool `yaml:"prepare,omitempty"`

	// only needed for packages that aren't in npm
	Name    string `yaml:"name,omitempty"`
	Version string `yaml:"version,omitempty"`

	LibC       []string `yaml:"libc,omitempty"`
	Deprecated string   `yaml:"deprecated,omitempty"`
}

// PackageResolution Various resolution strategies for packages
type PackageResolution struct {
	Type string `yaml:"type,omitempty"`
	// For npm or tarball
	Integrity string `yaml:"integrity,omitempty"`

	// For tarball
	Tarball string `yaml:"tarball,omitempty"`

	// For local directory
	Dir string `yaml:"directory,omitempty"`

	// For git repo
	Repo   string `yaml:"repo,omitempty"`
	Commit string `yaml:"commit,omitempty"`
}

// PatchFile represent a patch applied to a package
type PatchFile struct {
	Path string `yaml:"path"`
	Hash string `yaml:"hash"`
}

func isSupportedVersion(version float32) error {
	supportedVersions := []float32{5.3, 5.4}
	for _, supportedVersion := range supportedVersions {
		if version == supportedVersion {
			return nil
		}
	}
	return errors.Errorf("Unable to generate pnpm-lock.yaml with lockfileVersion: %f. Supported lockfile versions are %v", version, supportedVersions)
}

// DependencyMeta metadata for dependencies
type DependencyMeta struct {
	Injected bool   `yaml:"injected,omitempty"`
	Node     string `yaml:"node,omitempty"`
	Patch    string `yaml:"string,omitempty"`
}

// DecodePnpmLockfile parse a pnpm lockfile
func DecodePnpmLockfile(contents []byte) (*PnpmLockfile, error) {
	var lockfile PnpmLockfile
	if err := yaml.Unmarshal(contents, &lockfile); err != nil {
		return nil, errors.Wrap(err, "could not unmarshal lockfile: ")
	}

	if err := isSupportedVersion(lockfile.Version); err != nil {
		return nil, err
	}

	return &lockfile, nil
}

// ResolvePackage Given a package and version returns the key, resolved version, and if it was found
func (p *PnpmLockfile) ResolvePackage(name string, version string) (string, string, bool) {
	resolvedVersion, ok := p.resolveSpecifier(name, version)
	if !ok {
		return "", "", false
	}
	key := fmt.Sprintf("/%s/%s", name, resolvedVersion)
	if entry, ok := (p.Packages)[key]; ok {
		var version string
		if entry.Version != "" {
			version = entry.Version
		} else {
			version = resolvedVersion
		}
		return key, version, true
	}

	return "", "", false
}

// AllDependencies Given a lockfile key return all (dev/optional/peer) dependencies of that package
func (p *PnpmLockfile) AllDependencies(key string) (map[string]string, bool) {
	deps := map[string]string{}
	entry, ok := (p.Packages)[key]
	if !ok {
		return deps, false
	}

	for name, version := range entry.Dependencies {
		deps[name] = version
	}

	for name, version := range entry.OptionalDependencies {
		deps[name] = version
	}

	for name, version := range entry.PeerDependencies {
		deps[name] = version
	}

	return deps, true
}

// Subgraph Given a list of lockfile keys returns a Lockfile based off the original one that only contains the packages given
func (p *PnpmLockfile) Subgraph(packages []string) (Lockfile, error) {
	lockfilePackages := make(map[string]PackageSnapshot, len(packages))
	for _, key := range packages {
		entry, ok := p.Packages[key]
		if ok {
			lockfilePackages[key] = entry
		} else {
			return nil, fmt.Errorf("Unable to find lockfile entry for %s", key)
		}
	}

	lockfile := PnpmLockfile{
		Version:            p.Version,
		Importers:          p.Importers,
		Packages:           lockfilePackages,
		NeverBuiltDeps:     p.NeverBuiltDeps,
		OnlyBuiltDeps:      p.OnlyBuiltDeps,
		Overrides:          p.Overrides,
		PackageExtChecksum: p.PackageExtChecksum,
		PatchedDeps:        p.PatchedDeps,
	}

	return &lockfile, nil
}

// Encode encode the lockfile representation and write it to the given writer
func (p *PnpmLockfile) Encode(w io.Writer) error {
	if err := isSupportedVersion(p.Version); err != nil {
		return err
	}

	encoder := yaml.NewEncoder(w)
	encoder.SetIndent(2)

	if err := encoder.Encode(p); err != nil {
		return errors.Wrap(err, "unable to encode pnpm lockfile")
	}
	return nil
}

func (p *PnpmLockfile) resolveSpecifier(name string, specifier string) (string, bool) {
	// Check if the specifier is already a resolved version
	_, ok := p.Packages[fmt.Sprintf("/%s/%s", name, specifier)]
	if ok {
		return specifier, true
	}
	for workspacePkg, importer := range p.Importers {
		for pkgName, pkgSpecifier := range importer.Specifiers {
			if name == pkgName && specifier == pkgSpecifier {
				if resolvedVersion, ok := importer.Dependencies[name]; ok {
					return resolvedVersion, true
				}
				if resolvedVersion, ok := importer.DevDependencies[name]; ok {
					return resolvedVersion, true
				}
				if resolvedVersion, ok := importer.OptionalDependencies[name]; ok {
					return resolvedVersion, true
				}

				panic(fmt.Sprintf("Unable to find resolved version for %s@%s in %s", name, specifier, workspacePkg))
			}
		}
	}
	return "", false
}
