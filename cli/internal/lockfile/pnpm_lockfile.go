package lockfile

import (
	"fmt"
	"io"

	"github.com/pkg/errors"
	"github.com/vercel/turborepo/cli/internal/turbopath"
	"gopkg.in/yaml.v3"
)

// PnpmLockfile Go representation of the contents of 'pnpm-lock.yaml'
// Reference https://github.com/pnpm/pnpm/blob/main/packages/lockfile-types/src/index.ts
type PnpmLockfile struct {
	Version                   float32                    `yaml:"lockfileVersion"`
	NeverBuiltDependencies    []string                   `yaml:"neverBuiltDependencies,omitempty"`
	OnlyBuiltDependencies     []string                   `yaml:"onlyBuiltDependencies,omitempty"`
	Overrides                 map[string]string          `yaml:"overrides,omitempty"`
	PackageExtensionsChecksum string                     `yaml:"packageExtensionsChecksum,omitempty"`
	PatchedDependencies       map[string]PatchFile       `yaml:"patchedDependencies,omitempty"`
	Importers                 map[string]ProjectSnapshot `yaml:"importers"`
	Packages                  map[string]PackageSnapshot `yaml:"packages,omitempty"`
	Time                      map[string]string          `yaml:"time,omitempty"`
}

var _ Lockfile = (*PnpmLockfile)(nil)

// ProjectSnapshot Snapshot used to represent projects in the importers section
type ProjectSnapshot struct {
	Specifiers           map[string]string           `yaml:"specifiers"`
	Dependencies         map[string]string           `yaml:"dependencies,omitempty"`
	OptionalDependencies map[string]string           `yaml:"optionalDependencies,omitempty"`
	DevDependencies      map[string]string           `yaml:"devDependencies,omitempty"`
	DependenciesMeta     map[string]DependenciesMeta `yaml:"dependenciesMeta,omitempty"`
	PublishDirectory     string                      `yaml:"publishDirectory,omitempty"`
}

// PackageSnapshot Snapshot used to represent a package in the packages setion
type PackageSnapshot struct {
	Resolution PackageResolution `yaml:"resolution,flow"`
	ID         string            `yaml:"id,omitempty"`

	// only needed for packages that aren't in npm
	Name    string `yaml:"name,omitempty"`
	Version string `yaml:"version,omitempty"`

	Engines struct {
		Node string `yaml:"node"`
		NPM  string `yaml:"npm,omitempty"`
	} `yaml:"engines,omitempty,flow"`
	CPU  []string `yaml:"cpu,omitempty,flow"`
	Os   []string `yaml:"os,omitempty,flow"`
	LibC []string `yaml:"libc,omitempty"`

	Deprecated    string `yaml:"deprecated,omitempty"`
	HasBin        bool   `yaml:"hasBin,omitempty"`
	Prepare       bool   `yaml:"prepare,omitempty"`
	RequiresBuild bool   `yaml:"requiresBuild,omitempty"`

	BundledDependencies  []string          `yaml:"bundledDependencies,omitempty"`
	PeerDependencies     map[string]string `yaml:"peerDependencies,omitempty"`
	PeerDependenciesMeta map[string]struct {
		Optional bool `yaml:"optional"`
	} `yaml:"peerDependenciesMeta,omitempty"`

	Dependencies         map[string]string `yaml:"dependencies,omitempty"`
	OptionalDependencies map[string]string `yaml:"optionalDependencies,omitempty"`

	TransitivePeerDependencies []string `yaml:"transitivePeerDependencies,omitempty"`
	Dev                        bool     `yaml:"dev"`
	Optional                   bool     `yaml:"optional,omitempty"`
	Patched                    bool     `yaml:"patched,omitempty"`
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

// DependenciesMeta metadata for dependencies
type DependenciesMeta struct {
	Injected bool   `yaml:"injected,omitempty"`
	Node     string `yaml:"node,omitempty"`
	Patch    string `yaml:"patch,omitempty"`
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
func (p *PnpmLockfile) ResolvePackage(workspace string, name string, version string) (string, string, bool) {
	resolvedVersion, ok := p.resolveSpecifier(workspace, name, version)
	if !ok {
		return "", "", false
	}
	key := formatPnpmKey(name, resolvedVersion)
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
func (p *PnpmLockfile) Subgraph(workspacePackages []turbopath.AnchoredSystemPath, packages []string) (Lockfile, error) {
	lockfilePackages := make(map[string]PackageSnapshot, len(packages))
	for _, key := range packages {
		entry, ok := p.Packages[key]
		if ok {
			lockfilePackages[key] = entry
		} else {
			return nil, fmt.Errorf("Unable to find lockfile entry for %s", key)
		}
	}

	importers, err := pruneImporters(p.Importers, workspacePackages)
	if err != nil {
		return nil, err
	}

	lockfile := PnpmLockfile{
		Version:                   p.Version,
		Importers:                 importers,
		Packages:                  lockfilePackages,
		NeverBuiltDependencies:    p.NeverBuiltDependencies,
		OnlyBuiltDependencies:     p.OnlyBuiltDependencies,
		Overrides:                 p.Overrides,
		PackageExtensionsChecksum: p.PackageExtensionsChecksum,
		// TODO only the applicable patches should be copied to the subgraph
		// before we can implement this we need to be able to prune the pnpm section
		// of package.json otherwise installation will fail
		PatchedDependencies: p.PatchedDependencies,
	}

	return &lockfile, nil
}

// Prune imports to only those have all of their dependencies in the packages list
func pruneImporters(importers map[string]ProjectSnapshot, workspacePackages []turbopath.AnchoredSystemPath) (map[string]ProjectSnapshot, error) {
	prunedImporters := map[string]ProjectSnapshot{}

	// Copy over root level importer
	if root, ok := importers["."]; ok {
		prunedImporters["."] = root
	}

	for _, workspacePath := range workspacePackages {
		workspace := workspacePath.ToUnixPath().ToString()
		importer, ok := importers[workspace]

		if !ok {
			return nil, fmt.Errorf("Unable to find import entry for workspace package %s", workspace)
		}

		prunedImporters[workspace] = importer
	}

	return prunedImporters, nil
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

// Patches return a list of patches used in the lockfile
func (p *PnpmLockfile) Patches() []turbopath.AnchoredUnixPath {
	if len(p.PatchedDependencies) == 0 {
		return nil
	}
	patches := make([]turbopath.AnchoredUnixPath, len(p.PatchedDependencies))
	i := 0
	for _, patch := range p.PatchedDependencies {
		patches[i] = turbopath.AnchoredUnixPath(patch.Path)
		i++
	}
	return patches
}

func (p *PnpmLockfile) resolveSpecifier(workspace string, name string, specifier string) (string, bool) {
	// Check if the specifier is already a resolved version
	_, ok := p.Packages[formatPnpmKey(name, specifier)]
	if ok {
		return specifier, true
	}
	importer, ok := p.Importers[workspace]
	if !ok {
		return "", false
	}
	foundSpecifier, ok := importer.Specifiers[name]
	if !ok {
		return "", false
	}
	if foundSpecifier != specifier {
		return "", false
	}
	if resolvedVersion, ok := importer.Dependencies[name]; ok {
		return resolvedVersion, true
	}
	if resolvedVersion, ok := importer.DevDependencies[name]; ok {
		return resolvedVersion, true
	}
	if resolvedVersion, ok := importer.OptionalDependencies[name]; ok {
		return resolvedVersion, true
	}
	panic(fmt.Sprintf("Unable to find resolved version for %s@%s in %s", name, specifier, workspace))
}

func formatPnpmKey(name string, version string) string {
	return fmt.Sprintf("/%s/%s", name, version)
}
