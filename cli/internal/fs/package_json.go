package fs

import (
	"bytes"
	"encoding/json"
	"sync"

	"github.com/vercel/turbo/cli/internal/lockfile"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// PackageJSON represents NodeJS package.json
type PackageJSON struct {
	Name                 string            `json:"name"`
	Version              string            `json:"version"`
	Scripts              map[string]string `json:"scripts"`
	Dependencies         map[string]string `json:"dependencies"`
	DevDependencies      map[string]string `json:"devDependencies"`
	OptionalDependencies map[string]string `json:"optionalDependencies"`
	PeerDependencies     map[string]string `json:"peerDependencies"`
	PackageManager       string            `json:"packageManager"`
	Os                   []string          `json:"os"`
	Workspaces           Workspaces        `json:"workspaces"`
	Private              bool              `json:"private"`
	// Exact JSON object stored in package.json including unknown fields
	// During marshalling struct fields will take priority over raw fields
	RawJSON map[string]interface{} `json:"-"`

	// relative path from repo root to the package.json file
	PackageJSONPath turbopath.AnchoredSystemPath `json:"-"`
	// relative path from repo root to the package
	Dir                    turbopath.AnchoredSystemPath `json:"-"`
	InternalDeps           []string                     `json:"-"`
	UnresolvedExternalDeps map[string]string            `json:"-"`
	TransitiveDeps         []lockfile.Package           `json:"-"`
	LegacyTurboConfig      *TurboJSON                   `json:"turbo"`
	Mu                     sync.Mutex                   `json:"-"`
	ExternalDepsHash       string                       `json:"-"`
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

// ReadPackageJSON returns a struct of package.json
func ReadPackageJSON(path turbopath.AbsoluteSystemPath) (*PackageJSON, error) {
	b, err := path.ReadFile()
	if err != nil {
		return nil, err
	}
	return UnmarshalPackageJSON(b)
}

// UnmarshalPackageJSON decodes a byte slice into a PackageJSON struct
func UnmarshalPackageJSON(data []byte) (*PackageJSON, error) {
	var rawJSON map[string]interface{}
	if err := json.Unmarshal(data, &rawJSON); err != nil {
		return nil, err
	}

	pkgJSON := &PackageJSON{}
	if err := json.Unmarshal(data, &pkgJSON); err != nil {
		return nil, err
	}
	pkgJSON.RawJSON = rawJSON

	return pkgJSON, nil
}

// MarshalPackageJSON Serialize PackageJSON to a slice of bytes
func MarshalPackageJSON(pkgJSON *PackageJSON) ([]byte, error) {
	structuredContent, err := json.Marshal(pkgJSON)
	if err != nil {
		return nil, err
	}
	var structuredFields map[string]interface{}
	if err := json.Unmarshal(structuredContent, &structuredFields); err != nil {
		return nil, err
	}

	fieldsToSerialize := make(map[string]interface{}, len(pkgJSON.RawJSON))

	// copy pkgJSON.RawJSON
	for key, value := range pkgJSON.RawJSON {
		fieldsToSerialize[key] = value
	}

	for key, value := range structuredFields {
		if isEmpty(value) {
			delete(fieldsToSerialize, key)
		} else {
			fieldsToSerialize[key] = value
		}
	}

	var b bytes.Buffer
	encoder := json.NewEncoder(&b)
	encoder.SetEscapeHTML(false)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(fieldsToSerialize); err != nil {
		return nil, err
	}

	return b.Bytes(), nil
}

func isEmpty(value interface{}) bool {
	if value == nil {
		return true
	}
	switch s := value.(type) {
	case string:
		return s == ""
	case bool:
		return !s
	case []string:
		return len(s) == 0
	case map[string]interface{}:
		return len(s) == 0
	case Workspaces:
		return len(s) == 0
	default:
		// Assume any unknown types aren't empty
		return false
	}
}
