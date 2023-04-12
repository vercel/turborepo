package lockfile

import (
	"fmt"
	"io"
	"reflect"
	"sort"
	"strings"

	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/yaml"
)

// PnpmLockfile Go representation of the contents of 'pnpm-lock.yaml'
// Reference https://github.com/pnpm/pnpm/blob/main/packages/lockfile-types/src/index.ts
type PnpmLockfile struct {
	isV6 bool
	// Before 6.0 version was stored as a float, but as of 6.0+ it's a string
	Version                   interface{}                `yaml:"lockfileVersion"`
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
	// for v6 we omitempty
	// for pre v6 we *need* to omit the empty map
	Specifiers SpecifierMap `yaml:"specifiers,omitempty"`

	// The values of these maps will be string if lockfileVersion <6 or DependencyV6 if 6+
	Dependencies         map[string]yaml.Node `yaml:"dependencies,omitempty"`
	OptionalDependencies map[string]yaml.Node `yaml:"optionalDependencies,omitempty"`
	DevDependencies      map[string]yaml.Node `yaml:"devDependencies,omitempty"`

	DependenciesMeta map[string]DependenciesMeta `yaml:"dependenciesMeta,omitempty"`
	PublishDirectory string                      `yaml:"publishDirectory,omitempty"`
}

// SpecifierMap is a type wrapper that overrides IsZero for Golang's map
// to match the behavior that pnpm expects
type SpecifierMap map[string]string

// IsZero is used to check whether an object is zero to
// determine whether it should be omitted when marshaling
// with the omitempty flag.
func (m SpecifierMap) IsZero() bool {
	return m == nil
}

var _ (yaml.IsZeroer) = (*SpecifierMap)(nil)

// DependencyV6 are dependency entries for lockfileVersion 6.0+
type DependencyV6 struct {
	Specifier string `yaml:"specifier"`
	Version   string `yaml:"version"`
}

// Will try to find a resolution in any of the dependency fields
func (p *ProjectSnapshot) findResolution(dependency string) (DependencyV6, bool, error) {
	var getResolution func(yaml.Node) (DependencyV6, bool, error)
	if len(p.Specifiers) > 0 {
		getResolution = func(node yaml.Node) (DependencyV6, bool, error) {
			specifier, ok := p.Specifiers[dependency]
			if !ok {
				return DependencyV6{}, false, nil
			}
			var version string
			if err := node.Decode(&version); err != nil {
				return DependencyV6{}, false, err
			}
			return DependencyV6{Version: version, Specifier: specifier}, true, nil
		}
	} else {
		getResolution = func(node yaml.Node) (DependencyV6, bool, error) {
			var resolution DependencyV6
			if err := node.Decode(&resolution); err != nil {
				return DependencyV6{}, false, err
			}
			return resolution, true, nil
		}
	}
	if resolution, ok := p.Dependencies[dependency]; ok {
		return getResolution(resolution)
	}
	if resolution, ok := p.DevDependencies[dependency]; ok {
		return getResolution(resolution)
	}
	if resolution, ok := p.OptionalDependencies[dependency]; ok {
		return getResolution(resolution)
	}
	return DependencyV6{}, false, nil
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

func isSupportedVersion(version interface{}) error {
	switch version.(type) {
	case string:
		if version == "6.0" {
			return nil
		}
	case float64:
		if version == 5.3 || version == 5.4 {
			return nil
		}
	default:
		return fmt.Errorf("lockfileVersion of type %T is invalid", version)
	}
	supportedVersions := []string{"5.3", "5.4", "6.0"}
	return errors.Errorf("Unable to generate pnpm-lock.yaml with lockfileVersion: %s. Supported lockfile versions are %v", version, supportedVersions)
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
	err := yaml.Unmarshal(contents, &lockfile)
	if err != nil {
		return nil, errors.Wrap(err, "could not unmarshal lockfile: ")
	}

	switch lockfile.Version.(type) {
	case float64:
		lockfile.isV6 = false
	case string:
		lockfile.isV6 = true
	default:
		return nil, fmt.Errorf("Unexpected type of lockfileVersion: '%T', expected float64 or string", lockfile.Version)
	}
	return &lockfile, nil
}

// ResolvePackage Given a package and version returns the key, resolved version, and if it was found
func (p *PnpmLockfile) ResolvePackage(workspacePath turbopath.AnchoredUnixPath, name string, version string) (Package, error) {
	// Check if version is a key
	if _, ok := p.Packages[version]; ok {
		return Package{Key: version, Version: p.extractVersion(version), Found: true}, nil
	}

	resolvedVersion, ok, err := p.resolveSpecifier(workspacePath, name, version)
	if !ok || err != nil {
		return Package{}, err
	}
	key := p.formatKey(name, resolvedVersion)
	if entry, ok := (p.Packages)[key]; ok {
		var version string
		if entry.Version != "" {
			version = entry.Version
		} else {
			version = resolvedVersion
		}
		return Package{Key: key, Version: version, Found: true}, nil
	}

	if entry, ok := p.Packages[resolvedVersion]; ok {
		var version string
		if entry.Version != "" {
			version = entry.Version
		} else {
			// If there isn't a version field in the entry then the version is
			// encoded in the key and we can omit the name from the version.
			version = p.extractVersion(resolvedVersion)
		}
		return Package{Key: resolvedVersion, Version: version, Found: true}, nil
	}

	return Package{}, nil
}

// AllDependencies Given a lockfile key return all (dev/optional/peer) dependencies of that package
func (p *PnpmLockfile) AllDependencies(key string) (map[string]string, bool) {
	deps := map[string]string{}
	entry, ok := p.Packages[key]
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
				resolution, ok, err := importer.findResolution(dependency)
				if err != nil {
					return nil, errors.Wrapf(err, "Unable to decode reference to %s", dependency)
				}
				if !ok {
					return nil, fmt.Errorf("Unable to find %s other than reference in dependenciesMeta", dependency)
				}
				entry, ok := p.Packages[resolution.Version]
				if !ok {
					return nil, fmt.Errorf("Unable to find package entry for %s", resolution)
				}

				lockfilePackages[resolution.Version] = entry
			}
		}
	}

	lockfile := PnpmLockfile{
		Version:                   p.Version,
		Packages:                  lockfilePackages,
		NeverBuiltDependencies:    p.NeverBuiltDependencies,
		OnlyBuiltDependencies:     p.OnlyBuiltDependencies,
		Overrides:                 p.Overrides,
		PackageExtensionsChecksum: p.PackageExtensionsChecksum,
		PatchedDependencies:       p.prunePatches(p.PatchedDependencies, lockfilePackages),
		Importers:                 importers,
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

		// If a workspace has no dependencies *and* it is only depended on by the
		// workspace root it will not show up as an importer.
		if ok {
			prunedImporters[workspace] = importer
		}

	}

	return prunedImporters, nil
}

func (p *PnpmLockfile) prunePatches(patches map[string]PatchFile, packages map[string]PackageSnapshot) map[string]PatchFile {
	if len(patches) == 0 {
		return nil
	}

	patchPackages := make(map[string]PatchFile, len(patches))
	for dependency := range packages {
		if p.isV6 {
			// Internally pnpm partially converts the new path format to the old
			// format in order for existing parsing logic to work.
			dependency = convertNewToOldDepPath(dependency)
		}
		dp := parseDepPath(dependency)
		patchKey := fmt.Sprintf("%s@%s", dp.name, dp.version)

		if patch, ok := patches[patchKey]; ok && patch.Hash == dp.patchHash() {
			patchPackages[patchKey] = patch
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

// Patches return a list of patches used in the lockfile
func (p *PnpmLockfile) Patches() []turbopath.AnchoredUnixPath {
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

// GlobalChange checks if there are any differences between lockfiles that would completely invalidate
// the cache.
func (p *PnpmLockfile) GlobalChange(other Lockfile) bool {
	o, ok := other.(*PnpmLockfile)
	return !ok ||
		p.Version != o.Version ||
		p.PackageExtensionsChecksum != o.PackageExtensionsChecksum ||
		!reflect.DeepEqual(p.Overrides, o.Overrides) ||
		!reflect.DeepEqual(p.PatchedDependencies, o.PatchedDependencies)
}

func (p *PnpmLockfile) resolveSpecifier(workspacePath turbopath.AnchoredUnixPath, name string, specifier string) (string, bool, error) {
	pnpmWorkspacePath := workspacePath.ToString()
	if pnpmWorkspacePath == "" {
		// For pnpm, the root is named "."
		pnpmWorkspacePath = "."
	}
	importer, ok := p.Importers[pnpmWorkspacePath]
	if !ok {
		return "", false, fmt.Errorf("no workspace '%v' found in lockfile", workspacePath)
	}
	resolution, ok, err := importer.findResolution(name)
	if err != nil {
		return "", false, err
	}
	// Verify that the specifier in the importer matches the one given
	if !ok {
		// Check if the specifier is already a resolved version
		if _, ok := p.Packages[p.formatKey(name, specifier)]; ok {
			return specifier, true, nil
		}
		return "", false, fmt.Errorf("Unable to find resolved version for %s@%s in %s", name, specifier, workspacePath)
	}
	overrideSpecifier := p.applyOverrides(name, specifier)
	if resolution.Specifier != overrideSpecifier {
		if _, ok := p.Packages[p.formatKey(name, overrideSpecifier)]; ok {
			return overrideSpecifier, true, nil
		}
		return "", false, nil
	}
	return resolution.Version, true, nil
}

// Apply pnpm overrides to specifier, see https://pnpm.io/package_json#pnpmoverrides
// Note this is barebones support and will only supports global overrides
// future work will support semver ranges and selector filtering.
func (p *PnpmLockfile) applyOverrides(name string, specifier string) string {
	if len(p.Overrides) > 0 {
		if new, ok := p.Overrides[name]; ok {
			return new
		}
	}
	return specifier
}

// Formatter of the lockfile key given a package name and version
func (p *PnpmLockfile) formatKey(name string, version string) string {
	if p.isV6 {
		return fmt.Sprintf("/%s@%s", name, version)
	}
	return fmt.Sprintf("/%s/%s", name, version)
}

// Extracts version from lockfile key
func (p *PnpmLockfile) extractVersion(key string) string {
	if p.isV6 {
		key = convertNewToOldDepPath(key)
	}
	dp := parseDepPath(key)
	if dp.peerSuffix != "" {
		sep := ""
		if !p.isV6 {
			sep = "_"
		}
		return fmt.Sprintf("%s%s%s", dp.version, sep, dp.peerSuffix)
	}
	return dp.version
}

// Parsed representation of a pnpm lockfile key
type depPath struct {
	host       string
	name       string
	version    string
	peerSuffix string
}

func parseDepPath(dependency string) depPath {
	// See https://github.com/pnpm/pnpm/blob/185ab01adfc927ea23d2db08a14723bf51d0025f/packages/dependency-path/src/index.ts#L96
	var dp depPath
	parts := strings.Split(dependency, "/")
	shift := func() string {
		if len(parts) == 0 {
			return ""
		}
		val := parts[0]
		parts = parts[1:]
		return val
	}

	isAbsolute := dependency[0] != '/'
	// Skip leading '/'
	if !isAbsolute {
		shift()
	}

	if isAbsolute {
		dp.host = shift()
	}

	if len(parts) == 0 {
		return dp
	}

	if strings.HasPrefix(parts[0], "@") {
		dp.name = fmt.Sprintf("%s/%s", shift(), shift())
	} else {
		dp.name = shift()
	}

	version := strings.Join(parts, "/")
	if len(version) > 0 {
		var peerSuffixIndex int
		if strings.Contains(version, "(") && strings.HasSuffix(version, ")") {
			// v6 encodes peers deps using (peer=version)
			// also used to encode patches using (path_hash=hash)
			peerSuffixIndex = strings.Index(version, "(")
			dp.peerSuffix = version[peerSuffixIndex:]
			dp.version = version[0:peerSuffixIndex]
		} else {
			// pre v6 uses _ to separate version from peer dependencies
			// if a dependency is patched and has peer dependencies its version will
			// be encoded as version_patchHash_peerDepsHash
			peerSuffixIndex = strings.Index(version, "_")
			if peerSuffixIndex != -1 {
				dp.peerSuffix = version[peerSuffixIndex+1:]
				dp.version = version[0:peerSuffixIndex]
			}
		}
		if peerSuffixIndex == -1 {
			dp.version = version
		}
	}

	return dp
}

var _patchHashKey = "patch_hash="

func (d depPath) patchHash() string {
	if strings.HasPrefix(d.peerSuffix, "(") && strings.HasSuffix(d.peerSuffix, ")") {
		for _, part := range strings.Split(d.peerSuffix, "(") {
			if strings.HasPrefix(part, _patchHashKey) {
				// drop the enclosing ')'
				return part[len(_patchHashKey) : len(part)-1]
			}
		}
		// no patch entry found
		return ""
	}

	sepIndex := strings.Index(d.peerSuffix, "_")
	if sepIndex != -1 {
		return d.peerSuffix[:sepIndex]
	}
	// if a dependency just has a single suffix we can't tell if it's a patch or peer hash
	// return it in case it's a patch hash
	return d.peerSuffix
}

// Used to convert v6's dep path of /name@version to v5's /name/version
// See https://github.com/pnpm/pnpm/blob/185ab01adfc927ea23d2db08a14723bf51d0025f/lockfile/lockfile-file/src/experiments/inlineSpecifiersLockfileConverters.ts#L162
func convertNewToOldDepPath(newPath string) string {
	if len(newPath) > 2 && !strings.Contains(newPath[2:], "@") {
		return newPath
	}
	searchStartIndex := strings.Index(newPath, "/@") + 2
	index := strings.Index(newPath[searchStartIndex:], "@") + searchStartIndex
	if strings.Contains(newPath, "(") && index > strings.Index(newPath, "(") {
		return newPath
	}
	return fmt.Sprintf("%s/%s", newPath[0:index], newPath[index+1:])
}
