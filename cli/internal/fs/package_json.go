package fs

import (
	"encoding/json"
	"fmt"
	"io/ioutil"
	"reflect"
	"sync"

	"github.com/pascaldekloe/name"
)

// TurboCacheOptions are configuration for Turborepo cache

type TurboConfigJSON struct {
	Base               string   `json:"baseBranch,omitempty"`
	GlobalDependencies []string `json:"globalDependencies,omitempty"`
	TurboCacheOptions  string   `json:"cacheOptions,omitempty"`
	Outputs            []string `json:"outputs,omitempty"`
	RemoteCacheUrl     string   `json:"remoteCacheUrl,omitempty"`
	HashedEnv          []string `json:"env,omitempty"`
	Pipeline           map[string]Pipeline
}

// Camelcase string with optional args.
func Camelcase(s string, v ...interface{}) string {
	return name.CamelCase(fmt.Sprintf(s, v...), true)
}

var requiredFields = []string{"Name", "Version"}

type PPipeline struct {
	Outputs   *[]string `json:"outputs"`
	Cache     *bool     `json:"cache,omitempty"`
	DependsOn []string  `json:"dependsOn,omitempty"`
}

type Pipeline struct {
	Outputs   []string `json:"-"`
	Cache     *bool    `json:"cache,omitempty"`
	DependsOn []string `json:"dependsOn,omitempty"`
	HashedEnv []string `json:"env,omitempty"`
	PPipeline
}

func (c *Pipeline) UnmarshalJSON(data []byte) error {
	if err := json.Unmarshal(data, &c.PPipeline); err != nil {
		return err
	}
	// We actually need a nil value to be able to unmarshal the json
	// because we interpret the omission of outputs to be different
	// from an empty array. We can't use omitempty because it will
	// always unmarshal into an empty array which is not what we want.
	if c.PPipeline.Outputs != nil {
		c.Outputs = *c.PPipeline.Outputs
	}
	c.Cache = c.PPipeline.Cache
	c.DependsOn = c.PPipeline.DependsOn
	return nil
}

// PackageJSON represents NodeJS package.json
type PackageJSON struct {
	Name                   string            `json:"name,omitempty"`
	Version                string            `json:"version,omitempty"`
	Scripts                map[string]string `json:"scripts,omitempty"`
	Dependencies           map[string]string `json:"dependencies,omitempty"`
	DevDependencies        map[string]string `json:"devDependencies,omitempty"`
	OptionalDependencies   map[string]string `json:"optionalDependencies,omitempty"`
	PeerDependencies       map[string]string `json:"peerDependencies,omitempty"`
	Os                     []string          `json:"os,omitempty"`
	Workspaces             Workspaces        `json:"workspaces,omitempty"`
	Private                bool              `json:"private,omitempty"`
	PackageJSONPath        string
	Hash                   string
	Dir                    string
	InternalDeps           []string
	UnresolvedExternalDeps map[string]string
	ExternalDeps           []string
	SubLockfile            YarnLockfile
	Turbo                  TurboConfigJSON `json:"turbo"`
	Mu                     sync.Mutex
	FilesHash              string
	ExternalDepsHash       string
}

type Workspaces []string

type WorkspacesAlt struct {
	Packages []string `json:"packages,omitempty"`
}

func (r *Workspaces) UnmarshalJSON(data []byte) error {
	var tmp = &WorkspacesAlt{}
	if err := json.Unmarshal(data, &tmp); err == nil {
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

// Validate checks if provided package.json is valid.
func (p *PackageJSON) Validate() error {
	for _, fieldname := range requiredFields {
		value := getField(p, fieldname)
		if len(value) == 0 {
			return fmt.Errorf("'%s' field is required in package.json", fieldname)
		}
	}

	return nil
}

// getField returns struct field value by name.
func getField(i interface{}, fieldname string) string {
	value := reflect.ValueOf(i)
	field := reflect.Indirect(value).FieldByName(fieldname)
	return field.String()
}

// ReadPackageJSON returns a struct of package.json
func ReadPackageJSON(path string) (*PackageJSON, error) {
	b, err := ioutil.ReadFile(path)
	if err != nil {
		return nil, err
	}
	return Parse(b)
}
