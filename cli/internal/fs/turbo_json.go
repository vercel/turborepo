package fs

import (
	"encoding/json"
	"fmt"
	"log"
	"strings"

	"github.com/vercel/turborepo/cli/internal/util"
	"github.com/yosuke-furukawa/json5/encoding/json5"
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

// ReadTurboConfig toggles between reading from package.json or turbo.json to support early adopters.
func ReadTurboConfig(rootPath AbsolutePath, rootPackageJSON *PackageJSON) (*TurboJSON, error) {
	// If turbo.json exists, we use that
	// If pkg.Turbo exists, we warn about running the migration
	// Use pkg.Turbo if turbo.json doesn't exist
	// If neither exists, it's a fatal error
	turboJSONPath := rootPath.Join("turbo.json")

	if !turboJSONPath.FileExists() {
		if rootPackageJSON.LegacyTurboConfig == nil {
			// TODO: suggestion on how to create one
			return nil, fmt.Errorf("Could not find turbo.json. Follow directions at https://turborepo.org/docs/getting-started to create one")
		}
		log.Println("[WARNING] Turbo configuration now lives in \"turbo.json\". Migrate to turbo.json by running \"npx @turbo/codemod create-turbo-config\"")
		return rootPackageJSON.LegacyTurboConfig, nil
	}

	turboJSON, err := ReadTurboJSON(turboJSONPath)
	if err != nil {
		return nil, fmt.Errorf("turbo.json: %w", err)
	}

	if rootPackageJSON.LegacyTurboConfig != nil {
		log.Println("[WARNING] Ignoring legacy \"turbo\" key in package.json, using turbo.json instead. Consider deleting the \"turbo\" key from package.json")
		rootPackageJSON.LegacyTurboConfig = nil
	}

	return turboJSON, nil
}

// ReadTurboJSON reads turbo.json in to a struct
func ReadTurboJSON(path AbsolutePath) (*TurboJSON, error) {
	file, err := path.Open()
	if err != nil {
		return nil, err
	}

	var turboJSON *TurboJSON
	decoder := json5.NewDecoder(file)
	err = decoder.Decode(&turboJSON)
	if err != nil {
		println("error unmarshalling", err.Error())
		return nil, err
	}
	return turboJSON, nil
}

// RemoteCacheOptions is a struct for deserializing .remoteCache of turbo.json
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

// Pipeline is a struct for deserializing .pipeline in turbo.json
type Pipeline map[string]TaskDefinition

// GetTaskDefinition returns a TaskDefinition from a serialized definition in turbo.json
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

// TaskDefinition is a representation of the turbo.json pipeline for further computation.
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
