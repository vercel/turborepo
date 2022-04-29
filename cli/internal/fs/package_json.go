package fs

import (
	"encoding/json"
	"io/ioutil"
	"sync"
)

// PackageJSON represents NodeJS package.json
type PackageJSON struct {
	Name                   string            `json:"name,omitempty"`
	Version                string            `json:"version,omitempty"`
	Scripts                map[string]string `json:"scripts,omitempty"`
	Dependencies           map[string]string `json:"dependencies,omitempty"`
	DevDependencies        map[string]string `json:"devDependencies,omitempty"`
	OptionalDependencies   map[string]string `json:"optionalDependencies,omitempty"`
	PeerDependencies       map[string]string `json:"peerDependencies,omitempty"`
	PackageManager         string            `json:"packageManager,omitempty"`
	Os                     []string          `json:"os,omitempty"`
	Workspaces             Workspaces        `json:"workspaces,omitempty"`
	Private                bool              `json:"private,omitempty"`
	PackageJSONPath        string
	Dir                    string // relative path from repo root to the package
	InternalDeps           []string
	UnresolvedExternalDeps map[string]string
	ExternalDeps           []string
	SubLockfile            YarnLockfile
	LegacyTurboConfig      *TurboJSON `json:"turbo"`
	Mu                     sync.Mutex
	ExternalDepsHash       string
}

type Workspaces []string

type WorkspacesAlt struct {
	Packages []string `json:"packages,omitempty"`
}

func (r *Workspaces) UnmarshalJSON(data []byte) error {
	var tmp = &WorkspacesAlt{}
	if err := json.Unmarshal(data, tmp); err == nil {
		*r = Workspaces(tmp.Packages)
		return nil
	}
	var tempstr = []string{}
	if err := json.Unmarshal(data, &tempstr); err != nil {
		return err
	}
	*r = tempstr
	return nil
}

// Parse parses package.json payload and returns structure.
func Parse(payload []byte) (*PackageJSON, error) {
	var packagejson *PackageJSON
	err := json.Unmarshal(payload, &packagejson)
	return packagejson, err
}

// ReadPackageJSON returns a struct of package.json
func ReadPackageJSON(path string) (*PackageJSON, error) {
	b, err := ioutil.ReadFile(path)
	if err != nil {
		return nil, err
	}
	return Parse(b)
}
