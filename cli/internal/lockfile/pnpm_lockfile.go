package lockfile

import (
	"bytes"
	"fmt"
	"io"
	"strings"

	"text/template"

	"github.com/pkg/errors"
	"gopkg.in/yaml.v3"
)

const pnpmLockfileTemplate = `lockfileVersion: {{ .Version }}

importers:
{{ range $key, $val := .Importers }}
  {{ $key }}:
{{ displayProjectSnapshot $val }}
{{ end }}
packages:
{{ range $key, $val :=  .Packages }}
  {{ $key }}:
{{ displayPackageSnapshot $val }}
{{ end }}{{ if (eq .Version 5.4) }}
{{ end }}`

// PnpmLockfile Go representation of the contents of 'pnpm-lock.yaml'
// Reference https://github.com/pnpm/pnpm/blob/main/packages/lockfile-types/src/index.ts
type PnpmLockfile struct {
	Version   float32                    `yaml:"lockfileVersion"`
	Importers map[string]ProjectSnapshot `yaml:"importers"`
	// Keys are of the form '/$PACKAGE/$VERSION'
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
	HasBin bool `yaml:"hasBin,omitempty"`

	PeerDependencies     map[string]string `yaml:"peerDependencies,omitempty"`
	PeerDependenciesMeta map[string]struct {
		Optional bool `yaml:"optional"`
	} `yaml:"peerDependenciesMeta,omitempty"`
	Dependencies         map[string]string `yaml:"dependencies,omitempty"`
	OptionalDependencies map[string]string `yaml:"optionalDependencies,omitempty"`
	TransitivePeerDeps   []string          `yaml:"transitivePeerDependencies,omitempty"`
	BundledDependencies  []string          `yaml:"bundledDependencies,omitempty"`

	Dev           bool `yaml:"dev"`
	Optional      bool `yaml:"optional,omitempty"`
	RequiresBuild bool `yaml:"requiresBuild,omitempty"`
	Patched       bool `yaml:"patched,omitempty"`
	Prepare       bool `yaml:"prepare,omitempty"`

	// only needed for packages that aren't in npm
	Name    string `yaml:"name,omitempty"`
	Version string `yaml:"version,omitempty"`

	Os         []string `yaml:"os,omitempty"`
	CPU        []string `yaml:"cpu,omitempty"`
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
	if version != 5.3 && version != 5.4 {
		return errors.Errorf("Unable to generate pnpm-lock.yaml with lockfileVersion: %f", version)
	}
	return nil
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

// PossibleKeys Given a package name and version return all of the keys it might appear as in the lockfile
func (p *PnpmLockfile) PossibleKeys(name string, version string) []string {
	return []string{fmt.Sprintf("/%s/%s", name, version)}
}

// ResolvePackage Given a package and version returns the key, resolved version, and if it was found
func (p *PnpmLockfile) ResolvePackage(name string, version string) (string, string, bool) {
	resolvedVersion, ok := p.resolveSpecifier(name, version)
	if !ok {
		// @nocommit should we panic if not found?
		return "", "", false
	}
	for _, key := range p.PossibleKeys(name, resolvedVersion) {
		if entry, ok := (p.Packages)[key]; ok {
			return key, entry.Version, true
		}
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

// SubLockfile Given a list of lockfile keys returns a Lockfile based off the original one that only contains the packages given
func (p *PnpmLockfile) SubLockfile(packages []string) (Lockfile, error) {
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
	// will need custom serial logic since 5.3 vs 5.4 will change how we serialize
	if err := isSupportedVersion(p.Version); err != nil {
		return err
	}

	funcMap := template.FuncMap{
		"displayProjectSnapshot": displayProjectSnapshot,
		"displayPackageSnapshot": displayPackageSnapshot,
	}
	t, err := template.New("pnpm lockfile").Funcs(funcMap).Parse(pnpmLockfileTemplate)
	if err != nil {
		return errors.Wrap(err, "unable to parse pnpm-lock.yaml template")
	}

	return t.Execute(w, p)
}

func (p *PnpmLockfile) resolveSpecifier(name string, specifier string) (string, bool) {
	for _, importer := range p.Importers {
		for pkgName, pkgSpecifier := range importer.Specifiers {
			if name == pkgName && specifier == pkgSpecifier {
				resolvedVersion, ok := importer.Dependencies[name]
				if !ok {
					panic(fmt.Sprintf("Unable to find resolved version for %s@%s", name, specifier))
				}
				return resolvedVersion, true
			}
		}
	}
	return "", false
}

func displayProjectSnapshot(projectSnapshot ProjectSnapshot) string {
	var b bytes.Buffer
	encoder := yaml.NewEncoder(&b)
	encoder.SetIndent(2)
	if err := encoder.Encode(projectSnapshot); err != nil {
		panic("failed to encode importers")
	}

	return indentLines(b.String())
}

func displayPackageSnapshot(packageSnapshot PackageSnapshot) string {
	var b bytes.Buffer
	encoder := yaml.NewEncoder(&b)
	encoder.SetIndent(2)
	if err := encoder.Encode(packageSnapshot); err != nil {
		panic("failed to encode importers")
	}
	return indentLines(b.String())
}

func indentLines(text string) string {
	lines := strings.Split(strings.TrimRight(text, "\n"), "\n")
	indentedLines := make([]string, len(lines))
	for i, line := range lines {
		if line == "" {
			// Don't indent empty lines
			indentedLines[i] = ""
		} else {
			indentedLines[i] = fmt.Sprintf("    %s", line)
		}
	}

	return strings.Join(indentedLines, "\n")
}
