package fs

import (
	"encoding/json"
	"io/ioutil"
	"os"
	"sync"

	"github.com/vercel/turborepo/cli/internal/util"
	"github.com/yosuke-furukawa/json5/encoding/json5"
)

// TurboConfigJSON is the root turborepo configuration
type TurboConfigJSON struct {
	// Base Git branch
	Base string `json:"baseBranch,omitempty"`
	// Global root filesystem dependencies
	GlobalDependencies []string `json:"globalDependencies,omitempty"`
	// Pipeline is a map of Turbo pipeline entries which define the task graph
	// and cache behavior on a per task or per package-task basis.
	Pipeline PipelineConfig
	// Configuration options when interfacing with the remote cache
	RemoteCacheOptions RemoteCacheOptions `json:"remoteCache,omitempty"`
}

func ReadTurboConfigJSON(path string) (*TurboConfigJSON, error) {
	file, err := os.Open(path)
	if err != nil {
		return nil, err
	}

	var turboConfig *TurboConfigJSON
	decoder := json5.NewDecoder(file)
	err = decoder.Decode(&turboConfig)
	if err != nil {
		println("error unmarshalling", err.Error())
		return nil, err
	}
	return turboConfig, nil
}

type RemoteCacheOptions struct {
	TeamId    string `json:"teamId,omitempty"`
	Signature bool   `json:"signature,omitempty"`
}

type PPipeline struct {
	Outputs   *[]string `json:"outputs"`
	Cache     *bool     `json:"cache,omitempty"`
	DependsOn []string  `json:"dependsOn,omitempty"`
	Inputs    []string  `json:"inputs,omitempty"`
}

type PipelineConfig map[string]Pipeline

func (pc PipelineConfig) GetPipeline(taskID string) (Pipeline, bool) {
	if entry, ok := pc[taskID]; ok {
		return entry, true
	}
	_, task := util.GetPackageTaskFromId(taskID)
	entry, ok := pc[task]
	return entry, ok
}

type Pipeline struct {
	Outputs   []string `json:"-"`
	Cache     *bool    `json:"cache,omitempty"`
	DependsOn []string `json:"dependsOn,omitempty"`
	Inputs    []string `json:"inputs,omitempty"`
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
	c.Inputs = c.PPipeline.Inputs
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
	LegacyTurboConfig      *TurboConfigJSON `json:"turbo"`
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
