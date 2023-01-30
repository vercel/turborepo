package lockfile

import (
	"fmt"
	"io"
	"sort"
	"strings"

	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/yaml"
)

// PnpmLockfile Go representation of the contents of 'pnpm-lock.yaml'
// Reference https://github.com/pnpm/pnpm/blob/main/packages/lockfile-types/src/index.ts
type PnpmLockfile struct {
	PnpmLockfileBase `yaml:",inline"`
	Importers        map[string]ProjectSnapshot `yaml:"importers"`
}

// PnpmLockfileV6 Go representation of the contents of pnpm-lock.yaml version 6
type PnpmLockfileV6 struct {
	PnpmLockfileBase `yaml:",inline"`
	Importers        map[string]ProjectSnapshotV6 `yaml:"importers"`
}

// PnpmLockfileBase shared structure between lockfile versions
type PnpmLockfileBase struct {
	Version                   string                     `yaml:"lockfileVersion"`
	NeverBuiltDependencies    []string                   `yaml:"neverBuiltDependencies,omitempty"`
	OnlyBuiltDependencies     []string                   `yaml:"onlyBuiltDependencies,omitempty"`
	Overrides                 map[string]string          `yaml:"overrides,omitempty"`
	PackageExtensionsChecksum string                     `yaml:"packageExtensionsChecksum,omitempty"`
	PatchedDependencies       map[string]PatchFile       `yaml:"patchedDependencies,omitempty"`
	Packages                  map[string]PackageSnapshot `yaml:"packages,omitempty"`
	Time                      map[string]string          `yaml:"time,omitempty"`
}

var _ Lockfile = (*PnpmLockfile)(nil)
var _ Lockfile = (*PnpmLockfileV6)(nil)

// ProjectSnapshot Snapshot used to represent projects in the importers section
type ProjectSnapshot struct {
	Specifiers           map[string]string           `yaml:"specifiers"`
	Dependencies         map[string]string           `yaml:"dependencies,omitempty"`
	OptionalDependencies map[string]string           `yaml:"optionalDependencies,omitempty"`
	DevDependencies      map[string]string           `yaml:"devDependencies,omitempty"`
	DependenciesMeta     map[string]DependenciesMeta `yaml:"dependenciesMeta,omitempty"`
	PublishDirectory     string                      `yaml:"publishDirectory,omitempty"`
}

// ProjectSnapshotV6 Snapshot used to represent projects in the importers section
type ProjectSnapshotV6 struct {
	Specifiers           map[string]DependencyV6     `yaml:"specifiers,omitempty"`
	Dependencies         map[string]DependencyV6     `yaml:"dependencies,omitempty"`
	OptionalDependencies map[string]DependencyV6     `yaml:"optionalDependencies,omitempty"`
	DevDependencies      map[string]DependencyV6     `yaml:"devDependencies,omitempty"`
	DependenciesMeta     map[string]DependenciesMeta `yaml:"dependenciesMeta,omitempty"`
	PublishDirectory     string                      `yaml:"publishDirectory,omitempty"`
}

// DependencyV6 are dependency entries for lockfileVersion 6.0+
type DependencyV6 struct {
	Specifier string `yaml:"specifier"`
	Version   string `yaml:"version"`
}

// Will try to find a resolution in any of the dependency fields
func (p *ProjectSnapshot) findResolution(dependency string) (string, bool) {
	if resolution, ok := p.Dependencies[dependency]; ok {
		return resolution, true
	}
	if resolution, ok := p.DevDependencies[dependency]; ok {
		return resolution, true
	}
	if resolution, ok := p.OptionalDependencies[dependency]; ok {
		return resolution, true
	}
	return "", false
}

func (p *ProjectSnapshotV6) findResolution(dependency string) (string, bool) {
	if resolution, ok := p.Dependencies[dependency]; ok {
		return resolution.Version, true
	}
	if resolution, ok := p.DevDependencies[dependency]; ok {
		return resolution.Version, true
	}
	if resolution, ok := p.OptionalDependencies[dependency]; ok {
		return resolution.Version, true
	}
	return "", false
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

func isSupportedVersion(version string) error {
	supportedVersions := []string{"5.3", "5.4", "6.0"}
	for _, supportedVersion := range supportedVersions {
		if version == supportedVersion {
			return nil
		}
	}
	return errors.Errorf("Unable to generate pnpm-lock.yaml with lockfileVersion: %s. Supported lockfile versions are %v", version, supportedVersions)
}

// DependenciesMeta metadata for dependencies
type DependenciesMeta struct {
	Injected bool   `yaml:"injected,omitempty"`
	Node     string `yaml:"node,omitempty"`
	Patch    string `yaml:"patch,omitempty"`
}

// DecodePnpmLockfile parse a pnpm lockfile
func DecodePnpmLockfile(contents []byte) (Lockfile, error) {
	var legacyLockfile PnpmLockfile
	var lockfileV6 PnpmLockfileV6
	isV6 := false
	// if this fails try v6 schema
	err := yaml.Unmarshal(contents, &legacyLockfile)
	if err != nil {
		err = yaml.Unmarshal(contents, &lockfileV6)
		isV6 = true
	}
	if err != nil {
		return nil, errors.Wrap(err, "could not unmarshal lockfile: ")
	}

	var version string
	if isV6 {
		version = lockfileV6.Version
	} else {
		version = legacyLockfile.Version
	}
	if err := isSupportedVersion(version); err != nil {
		return nil, err
	}

	if isV6 {
		return &lockfileV6, nil
	}
	return &legacyLockfile, nil
}

// ResolvePackage Given a package and version returns the key, resolved version, and if it was found
func (p *PnpmLockfile) ResolvePackage(workspacePath turbopath.AnchoredUnixPath, name string, version string) (Package, error) {
	// Check if version is a key
	if _, ok := p.Packages[version]; ok {
		return Package{Key: version, Version: getVersionFromKey(version), Found: true}, nil
	}

	resolvedVersion, ok, err := p.resolveSpecifier(workspacePath, name, version)
	if !ok || err != nil {
		return Package{}, err
	}
	key := formatPnpmKey(name, resolvedVersion)
	if entry, ok := (p.Packages)[key]; ok {
		var version string
		if entry.Version != "" {
			version = entry.Version
		} else {
			version = resolvedVersion
		}
		return Package{Key: key, Version: version, Found: true}, nil
	}

	return Package{}, nil
}

// ResolvePackage Given a package and version returns the key, resolved version, and if it was found
func (p *PnpmLockfileV6) ResolvePackage(workspacePath turbopath.AnchoredUnixPath, name string, version string) (Package, error) {
	// Check if version is a key
	if _, ok := p.Packages[version]; ok {
		return Package{Key: version, Version: getVersionFromKeyV6(version), Found: true}, nil
	}

	resolvedVersion, ok, err := p.resolveSpecifier(workspacePath, name, version)
	if !ok || err != nil {
		return Package{}, err
	}
	key := formatPnpmKeyV6(name, resolvedVersion)
	if entry, ok := (p.Packages)[key]; ok {
		var version string
		if entry.Version != "" {
			version = entry.Version
		} else {
			version = resolvedVersion
		}
		return Package{Key: key, Version: version, Found: true}, nil
	}

	return Package{}, nil
}

func (p *PnpmLockfileBase) allDependencies(key string) (map[string]string, bool) {
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

	// Peer dependencies appear in the Dependencies map resolved

	return deps, true
}

// AllDependencies Given a lockfile key return all (dev/optional/peer) dependencies of that package
func (p *PnpmLockfile) AllDependencies(key string) (map[string]string, bool) {
	return p.PnpmLockfileBase.allDependencies(key)
}

// AllDependencies Given a lockfile key return all (dev/optional/peer) dependencies of that package
func (p *PnpmLockfileV6) AllDependencies(key string) (map[string]string, bool) {
	return p.PnpmLockfileBase.allDependencies(key)
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

	for _, importer := range importers {
		for dependency, meta := range importer.DependenciesMeta {
			if meta.Injected {
				resolution, ok := importer.findResolution(dependency)
				if !ok {
					return nil, fmt.Errorf("Unable to find %s other than reference in dependenciesMeta", dependency)
				}
				entry, ok := p.Packages[resolution]
				if !ok {
					return nil, fmt.Errorf("Unable to find package entry for %s", resolution)
				}

				lockfilePackages[resolution] = entry
			}
		}
	}

	base := PnpmLockfileBase{
		Version:                   p.Version,
		Packages:                  lockfilePackages,
		NeverBuiltDependencies:    p.NeverBuiltDependencies,
		OnlyBuiltDependencies:     p.OnlyBuiltDependencies,
		Overrides:                 p.Overrides,
		PackageExtensionsChecksum: p.PackageExtensionsChecksum,
		PatchedDependencies:       prunePatches(p.PatchedDependencies, lockfilePackages),
	}
	lockfile := PnpmLockfile{
		Importers:        importers,
		PnpmLockfileBase: base,
	}

	return &lockfile, nil
}

// Subgraph Given a list of lockfile keys returns a Lockfile based off the original one that only contains the packages given
func (p *PnpmLockfileV6) Subgraph(workspacePackages []turbopath.AnchoredSystemPath, packages []string) (Lockfile, error) {
	lockfilePackages := make(map[string]PackageSnapshot, len(packages))
	for _, key := range packages {
		entry, ok := p.Packages[key]
		if ok {
			lockfilePackages[key] = entry
		} else {
			return nil, fmt.Errorf("Unable to find lockfile entry for %s", key)
		}
	}

	importers, err := pruneImportersV6(p.Importers, workspacePackages)
	if err != nil {
		return nil, err
	}

	for _, importer := range importers {
		for dependency, meta := range importer.DependenciesMeta {
			if meta.Injected {
				resolution, ok := importer.findResolution(dependency)
				if !ok {
					return nil, fmt.Errorf("Unable to find %s other than reference in dependenciesMeta", dependency)
				}
				entry, ok := p.Packages[resolution]
				if !ok {
					return nil, fmt.Errorf("Unable to find package entry for %s", resolution)
				}

				lockfilePackages[resolution] = entry
			}
		}
	}

	base := PnpmLockfileBase{
		Version:                   p.Version,
		Packages:                  lockfilePackages,
		NeverBuiltDependencies:    p.NeverBuiltDependencies,
		OnlyBuiltDependencies:     p.OnlyBuiltDependencies,
		Overrides:                 p.Overrides,
		PackageExtensionsChecksum: p.PackageExtensionsChecksum,
		PatchedDependencies:       prunePatchesV6(p.PatchedDependencies, lockfilePackages),
	}
	lockfile := PnpmLockfileV6{
		Importers:        importers,
		PnpmLockfileBase: base,
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

func pruneImportersV6(importers map[string]ProjectSnapshotV6, workspacePackages []turbopath.AnchoredSystemPath) (map[string]ProjectSnapshotV6, error) {
	prunedImporters := map[string]ProjectSnapshotV6{}

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

func prunePatches(patches map[string]PatchFile, packages map[string]PackageSnapshot) map[string]PatchFile {
	if len(patches) == 0 {
		return nil
	}

	patchPackages := make(map[string]PatchFile, len(patches))
	for dependency, entry := range patches {
		// The name for patches is of the form name@version
		// https://github.com/pnpm/pnpm/blob/2895389ae1f2bf7346e140c017f495aa47186eba/packages/plugin-commands-patching/src/patchCommit.ts#L38
		lastAt := strings.LastIndex(dependency, "@")
		if lastAt == -1 {
			panic(fmt.Sprintf("No '@' found in patch key: %s", dependency))
		}
		name := strings.Replace(dependency[:lastAt], "/", "-", 1)
		version := dependency[lastAt+1:]

		dependencyString := fmt.Sprintf("%s_%s", formatPnpmKey(name, version), entry.Hash)
		_, inPackages := packages[dependencyString]
		if inPackages {
			patchPackages[dependency] = entry
		}
	}

	return patchPackages
}

func prunePatchesV6(patches map[string]PatchFile, packages map[string]PackageSnapshot) map[string]PatchFile {
	if len(patches) == 0 {
		return nil
	}

	patchPackages := make(map[string]PatchFile, len(patches))
	for dependency, entry := range patches {
		_, inPackages := packages["/"+dependency]
		if inPackages {
			patchPackages[dependency] = entry
		}
	}

	return patchPackages
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

// Encode encode the lockfile representation and write it to the given writer
func (p *PnpmLockfileV6) Encode(w io.Writer) error {
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
	return p.PnpmLockfileBase.patches()
}

// Patches return a list of patches used in the lockfile
func (p *PnpmLockfileV6) Patches() []turbopath.AnchoredUnixPath {
	return p.PnpmLockfileBase.patches()
}

func (p *PnpmLockfileBase) patches() []turbopath.AnchoredUnixPath {
	if len(p.PatchedDependencies) == 0 {
		return nil
	}
	patches := make([]string, len(p.PatchedDependencies))
	i := 0
	for _, patch := range p.PatchedDependencies {
		patches[i] = patch.Path
		i++
	}
	sort.Strings(patches)

	patchPaths := make([]turbopath.AnchoredUnixPath, len(p.PatchedDependencies))
	for i, patch := range patches {
		patchPaths[i] = turbopath.AnchoredUnixPath(patch)
	}
	return patchPaths
}

func (p *PnpmLockfile) resolveSpecifier(workspacePath turbopath.AnchoredUnixPath, name string, specifier string) (string, bool, error) {
	// Check if the specifier is already a resolved version
	_, ok := p.Packages[formatPnpmKey(name, specifier)]
	if ok {
		return specifier, true, nil
	}
	pnpmWorkspacePath := workspacePath.ToString()
	if pnpmWorkspacePath == "" {
		// For pnpm, the root is named "."
		pnpmWorkspacePath = "."
	}
	importer, ok := p.Importers[pnpmWorkspacePath]
	if !ok {
		return "", false, fmt.Errorf("no workspace '%v' found in lockfile", workspacePath)
	}
	foundSpecifier, ok := importer.Specifiers[name]
	if !ok {
		return "", false, nil
	}
	if foundSpecifier != specifier {
		return "", false, nil
	}
	if resolvedVersion, ok := importer.Dependencies[name]; ok {
		return resolvedVersion, true, nil
	}
	if resolvedVersion, ok := importer.DevDependencies[name]; ok {
		return resolvedVersion, true, nil
	}
	if resolvedVersion, ok := importer.OptionalDependencies[name]; ok {
		return resolvedVersion, true, nil
	}
	return "", false, fmt.Errorf("Unable to find resolved version for %s@%s in %s", name, specifier, workspacePath)
}

func (p *PnpmLockfileV6) resolveSpecifier(workspacePath turbopath.AnchoredUnixPath, name string, specifier string) (string, bool, error) {
	// Check if the specifier is already a resolved version
	_, ok := p.Packages[formatPnpmKeyV6(name, specifier)]
	if ok {
		return specifier, true, nil
	}
	pnpmWorkspacePath := workspacePath.ToString()
	if pnpmWorkspacePath == "" {
		// For pnpm, the root is named "."
		pnpmWorkspacePath = "."
	}
	importer, ok := p.Importers[pnpmWorkspacePath]
	if !ok {
		return "", false, fmt.Errorf("no workspace '%v' found in lockfile", workspacePath)
	}
	if resolved, ok := importer.Dependencies[name]; ok {
		return resolved.Version, true, nil
	}
	if resolved, ok := importer.DevDependencies[name]; ok {
		return resolved.Version, true, nil
	}
	if resolved, ok := importer.OptionalDependencies[name]; ok {
		return resolved.Version, true, nil
	}
	return "", false, fmt.Errorf("Unable to find resolved version for %s@%s in %s", name, specifier, workspacePath)
}

func formatPnpmKey(name string, version string) string {
	return fmt.Sprintf("/%s/%s", name, version)
}

func formatPnpmKeyV6(name string, version string) string {
	return fmt.Sprintf("/%s@%s", name, version)
}

func getVersionFromKey(key string) string {
	atIndex := strings.LastIndex(key, "/")
	if atIndex == -1 || len(key) == atIndex+1 {
		return ""
	}
	return key[atIndex+1:]
}

func getVersionFromKeyV6(key string) string {
	atIndex := strings.LastIndex(key, "@")
	if atIndex == -1 || len(key) == atIndex+1 {
		return ""
	}
	return key[atIndex+1:]
}
