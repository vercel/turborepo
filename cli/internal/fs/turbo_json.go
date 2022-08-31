package fs

import (
	"encoding/json"
	"fmt"
	"io/ioutil"
	"log"
	"strings"

	"github.com/vercel/turborepo/cli/internal/turbopath"
	"github.com/vercel/turborepo/cli/internal/util"
	"muzzammil.xyz/jsonc"
)

// TurboJSON is the root turborepo configuration
type TurboJSON struct {
	// Global root filesystem dependencies
	GlobalDependencies []string `json:"globalDependencies,omitempty"`
	// Pipeline is a map of Turbo pipeline entries which define the task graph
	// and cache behavior on a per task or per package-task basis.
	Pipeline Pipeline
	// Configuration options when interfacing with the remote cache
	RemoteCacheOptions RemoteCacheOptions `json:"remoteCache,omitempty"`
}

const configFile = "turbo.json"

// ReadTurboConfig toggles between reading from package.json or the configFile to support early adopters.
func ReadTurboConfig(rootPath turbopath.AbsolutePath, rootPackageJSON *PackageJSON) (*TurboJSON, error) {
	// If the configFile exists, we use that
	// If pkg.Turbo exists, we warn about running the migration
	// Use pkg.Turbo if the configFile doesn't exist
	// If neither exists, it's a fatal error
	turboJSONPath := rootPath.Join(configFile)

	if !turboJSONPath.FileExists() {
		if rootPackageJSON.LegacyTurboConfig == nil {
			// TODO: suggestion on how to create one
			return nil, fmt.Errorf("Could not find %s. Follow directions at https://turborepo.org/docs/getting-started to create one", configFile)
		}
		log.Printf("[WARNING] Turbo configuration now lives in \"%s\". Migrate to %s by running \"npx @turbo/codemod create-turbo-config\"", configFile, configFile)
		return rootPackageJSON.LegacyTurboConfig, nil
	}

	turboJSON, err := readTurboJSON(turboJSONPath)
	if err != nil {
		return nil, fmt.Errorf("%s: %w", configFile, err)
	}

	if rootPackageJSON.LegacyTurboConfig != nil {
		log.Printf("[WARNING] Ignoring legacy \"turbo\" key in package.json, using %s instead. Consider deleting the \"turbo\" key from package.json", configFile)
		rootPackageJSON.LegacyTurboConfig = nil
	}

	return turboJSON, nil
}

// readTurboJSON reads the configFile in to a struct
func readTurboJSON(path turbopath.AbsolutePath) (*TurboJSON, error) {
	file, err := path.Open()
	if err != nil {
		return nil, err
	}
	var turboJSON *TurboJSON
	data, err := ioutil.ReadAll(file)
	if err != nil {
		return nil, err
	}
	err = jsonc.Unmarshal(data, &turboJSON)
	if err != nil {
		println("error unmarshalling", err.Error())
		return nil, err
	}
	return turboJSON, nil
}

// RemoteCacheOptions is a struct for deserializing .remoteCache of configFile
type RemoteCacheOptions struct {
	TeamID    string `json:"teamId,omitempty"`
	Signature bool   `json:"signature,omitempty"`
}

type pipelineJSON struct {
	Outputs    *[]string           `json:"outputs"`
	Cache      *bool               `json:"cache,omitempty"`
	DependsOn  []string            `json:"dependsOn,omitempty"`
	Inputs     []string            `json:"inputs,omitempty"`
	OutputMode util.TaskOutputMode `json:"outputMode,omitempty"`
}

// Pipeline is a struct for deserializing .pipeline in configFile
type Pipeline map[string]TaskDefinition

// GetTaskDefinition returns a TaskDefinition from a serialized definition in configFile
func (pc Pipeline) GetTaskDefinition(taskID string) (TaskDefinition, bool) {
	if entry, ok := pc[taskID]; ok {
		return entry, true
	}
	_, task := util.GetPackageTaskFromId(taskID)
	entry, ok := pc[task]
	return entry, ok
}

// HasTask returns true if the given task is defined in the pipeline, either directly or
// via a package task (`pkg#task`)
func (pc Pipeline) HasTask(task string) bool {
	for key := range pc {
		if key == task {
			return true
		}
		if util.IsPackageTask(key) {
			_, taskName := util.GetPackageTaskFromId(key)
			if taskName == task {
				return true
			}
		}
	}
	return false
}

// TaskDefinition is a representation of the configFile pipeline for further computation.
type TaskDefinition struct {
	Outputs                 []string
	ShouldCache             bool
	EnvVarDependencies      []string
	TopologicalDependencies []string
	TaskDependencies        []string
	Inputs                  []string
	OutputMode              util.TaskOutputMode
}

const (
	envPipelineDelimiter         = "$"
	topologicalPipelineDelimiter = "^"
)

var defaultOutputs = []string{"dist/**/*", "build/**/*"}

// UnmarshalJSON deserializes JSON into a TaskDefinition
func (c *TaskDefinition) UnmarshalJSON(data []byte) error {
	rawPipeline := &pipelineJSON{}
	if err := json.Unmarshal(data, &rawPipeline); err != nil {
		return err
	}

	// We actually need a nil value to be able to unmarshal the json
	// because we interpret the omission of outputs to be different
	// from an empty array. We can't use omitempty because it will
	// always unmarshal into an empty array which is not what we want.
	if rawPipeline.Outputs != nil {
		c.Outputs = *rawPipeline.Outputs
	} else {
		c.Outputs = defaultOutputs
	}
	if rawPipeline.Cache == nil {
		c.ShouldCache = true
	} else {
		c.ShouldCache = *rawPipeline.Cache
	}
	c.EnvVarDependencies = []string{}
	c.TopologicalDependencies = []string{}
	c.TaskDependencies = []string{}
	for _, dependency := range rawPipeline.DependsOn {
		if strings.HasPrefix(dependency, envPipelineDelimiter) {
			c.EnvVarDependencies = append(c.EnvVarDependencies, strings.TrimPrefix(dependency, envPipelineDelimiter))
		} else if strings.HasPrefix(dependency, topologicalPipelineDelimiter) {
			c.TopologicalDependencies = append(c.TopologicalDependencies, strings.TrimPrefix(dependency, topologicalPipelineDelimiter))
		} else {
			c.TaskDependencies = append(c.TaskDependencies, dependency)
		}
	}
	c.Inputs = rawPipeline.Inputs
	c.OutputMode = rawPipeline.OutputMode
	return nil
}
