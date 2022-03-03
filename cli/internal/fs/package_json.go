package fs

import (
	"encoding/json"
	"io/ioutil"
	"os"
	"sync"

	"github.com/yosuke-furukawa/json5/encoding/json5"
)

// TurboConfigJSON is the root turborepo configuration
type TurboConfigJSON struct {
	JsonSchema string `json:"$schema"`
	// Base is the branch or your git repository. Git is used by turbo in its hashing algorithm
	// and --since CLI flag.
	Base string `json:"baseBranch,omitempty" jsonschema:"default=origin/master,example=origin/main"`
	// GlobalDependencies is a list of globs and environment variables for implicit global hash dependencies.
	// Environment variables should be prefixed with $ (e.g. $GITHUB_TOKEN).
	//
	// Any other entry without this prefix, will be considered filesystem glob. The
	// contents of these files will be included in the global hashing algorithm and affect
	// the hashes of all tasks.
	//
	// This is useful for busting the cache based on .env files (not in Git), environment
	// variables, or any root level file that impacts package tasks (but are not represented
	// in the traditional dependency graph
	//
	// (e.g. a root tsconfig.json, jest.config.js, .eslintrc, etc.)).
	GlobalDependencies []string `json:"globalDependencies,omitempty"`
	TurboCacheOptions  string   `json:"cacheOptions,omitempty"`
	Outputs            []string `json:"outputs,omitempty"`
	// RemoteCacheUrl is the Remote Cache API URL
	RemoteCacheUrl string `json:"remoteCacheUrl,omitempty"`
	// Pipeline is an object representing the task dependency graph of your project. turbo interprets
	// these conventions to properly schedule, execute, and cache the outputs of tasks in
	// your project.
	//
	// Each key in this object is the name of a task that can be executed by turbo run. If turbo finds a workspace
	// package with a package.json scripts object with a matching key, it will apply the
	// pipeline task configuration to that NPM script during execution. This allows you to
	// use pipeline to set conventions across your entire Turborepo.
	Pipeline map[string]Pipeline `json:"pipeline"`
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

type PPipeline struct {
	// The set of glob patterns of a task's cacheable filesystem outputs.
	//
	// Note: turbo automatically logs stderr/stdout to .turbo/run-<task>.log. This file is
	// always treated as a cacheable artifact and never needs to be specified.
	//
	// Passing an empty array can be used to tell turbo that a task is a side-effect and
	// thus doesn't emit any filesystem artifacts (e.g. like a linter), but you still want
	// to cache its logs (and treat them like an artifact).
	Outputs *[]string `json:"outputs" jsonschema:"default=dist/**,default=build/**"`
	// Whether or not to cache the task outputs. Setting cache to false is useful for daemon
	// or long-running "watch" or development mode tasks that you don't want to cache.
	Cache *bool `json:"cache,omitempty" jsonschema:"default=true"`
	// The list of tasks and environment variables that this task depends on.
	//
	// Prefixing an item in dependsOn with a ^ tells turbo that this pipeline task depends
	// on the package's topological dependencies completing the task with the ^ prefix first
	// (e.g. "a package's build tasks should only run once all of its dependencies and
	// devDependencies have completed their own build commands").
	//
	// Items in dependsOn without ^ prefix, express the relationships between tasks at the
	// package level (e.g. "a package's test and lint commands depend on build being
	// completed first").
	//
	// Prefixing an item in dependsOn with a $ tells turbo that this pipeline task depends
	// the value of that environment variable.
	DependsOn []string `json:"dependsOn,omitempty"`
}

type Pipeline struct {
	Outputs   []string `json:"-"`
	Cache     *bool    `json:"cache,omitempty"`
	DependsOn []string `json:"dependsOn,omitempty"`
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
	PackageManager         string            `json:"packageManager,omitempty"`
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
	LegacyTurboConfig      *TurboConfigJSON `json:"turbo"`
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
