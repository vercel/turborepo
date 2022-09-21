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

const (
	configFile                   = "turbo.json"
	envPipelineDelimiter         = "$"
	topologicalPipelineDelimiter = "^"
)

var defaultOutputs = []string{"dist/**/*", "build/**/*"}

type rawTurboJSON struct {
	// Global root filesystem dependencies
	GlobalDependencies []string `json:"globalDependencies,omitempty"`
	// Global env
	GlobalEnv []string `json:"globalEnv,omitempty"`
	// Pipeline is a map of Turbo pipeline entries which define the task graph
	// and cache behavior on a per task or per package-task basis.
	Pipeline Pipeline
	// Configuration options when interfacing with the remote cache
	RemoteCacheOptions RemoteCacheOptions `json:"remoteCache,omitempty"`
}

// TurboJSON is the root turborepo configuration
type TurboJSON struct {
	GlobalDeps         []string
	GlobalEnv          []string
	Pipeline           Pipeline
	RemoteCacheOptions RemoteCacheOptions
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
	Env        []string            `json:"env,omitempty"`
}

// Pipeline is a struct for deserializing .pipeline in configFile
type Pipeline map[string]TaskDefinition

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

// ReadTurboConfig toggles between reading from package.json or the configFile to support early adopters.
func ReadTurboConfig(rootPath turbopath.AbsoluteSystemPath, rootPackageJSON *PackageJSON) (*TurboJSON, error) {

	turboJSONPath := rootPath.UnsafeJoin(configFile)

	// Check if turbo key in package.json exists
	hasLegacyConfig := rootPackageJSON.LegacyTurboConfig != nil

	// If the configFile exists, use that
	if turboJSONPath.FileExists() {
		turboJSON, err := readTurboJSON(turboJSONPath)
		if err != nil {
			return nil, fmt.Errorf("%s: %w", configFile, err)
		}

		// If pkg.Turbo exists, log a warning and delete it from the representation
		// TODO: turn off this warning eventually
		if hasLegacyConfig {
			log.Printf("[WARNING] Ignoring \"turbo\" key in package.json, using %s instead.", configFile)
			rootPackageJSON.LegacyTurboConfig = nil
		}

		return turboJSON, nil
	}

	// Use pkg.Turbo if the configFile doesn't exist and we want the fallback feature
	// TODO: turn this fallback off eventually
	if hasLegacyConfig {
		log.Printf("[DEPRECATED] \"turbo\" in package.json is deprecated. Migrate to %s by running \"npx @turbo/codemod create-turbo-config\"\n", configFile)
		return rootPackageJSON.LegacyTurboConfig, nil
	}

	// If there's no turbo.json and no turbo key in package.json, return an error.
	return nil, fmt.Errorf("Could not find %s. Follow directions at https://turborepo.org/docs/getting-started to create one", configFile)
}

// readTurboJSON reads the configFile in to a struct
func readTurboJSON(path turbopath.AbsoluteSystemPath) (*TurboJSON, error) {
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
		return nil, err
	}

	return turboJSON, nil
}

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

	envVarDependencies := make(util.Set)
	c.TopologicalDependencies = []string{}
	c.TaskDependencies = []string{}

	for _, dependency := range rawPipeline.DependsOn {
		if strings.HasPrefix(dependency, envPipelineDelimiter) {
			envVarDependencies.Add(strings.TrimPrefix(dependency, envPipelineDelimiter))
		} else if strings.HasPrefix(dependency, topologicalPipelineDelimiter) {
			c.TopologicalDependencies = append(c.TopologicalDependencies, strings.TrimPrefix(dependency, topologicalPipelineDelimiter))
		} else {
			c.TaskDependencies = append(c.TaskDependencies, dependency)
		}
	}

	// Append env key into EnvVarDependencies
	for _, value := range rawPipeline.Env {
		if strings.HasPrefix(value, envPipelineDelimiter) {
			// Hard error to help people specify this correctly during migration.
			// TODO: Remove this error after we have run summary.
			return fmt.Errorf("You specified \"%s\" in the \"env\" key. You should not prefix your environment variables with \"$\"", value)
		}

		envVarDependencies.Add(value)
	}

	c.EnvVarDependencies = envVarDependencies.UnsafeListOfStrings()
	c.Inputs = rawPipeline.Inputs
	c.OutputMode = rawPipeline.OutputMode
	return nil
}

// UnmarshalJSON deserializes TurboJSON objects into struct
func (c *TurboJSON) UnmarshalJSON(data []byte) error {
	raw := &rawTurboJSON{}
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}

	envVarDependencies := make(util.Set)
	globalFileDependencies := make(util.Set)

	for _, value := range raw.GlobalEnv {
		if strings.HasPrefix(value, envPipelineDelimiter) {
			// Hard error to help people specify this correctly during migration.
			// TODO: Remove this error after we have run summary.
			return fmt.Errorf("You specified \"%s\" in the \"env\" key. You should not prefix your environment variables with \"%s\"", value, envPipelineDelimiter)
		}

		envVarDependencies.Add(value)
	}

	for _, value := range raw.GlobalDependencies {
		if strings.HasPrefix(value, envPipelineDelimiter) {
			envVarDependencies.Add(strings.TrimPrefix(value, envPipelineDelimiter))
		} else {
			globalFileDependencies.Add(value)
		}
	}

	// turn the set into an array and assign to the TurboJSON struct fields.
	c.GlobalEnv = envVarDependencies.UnsafeListOfStrings()
	c.GlobalDeps = globalFileDependencies.UnsafeListOfStrings()

	// copy these over, we don't need any changes here.
	c.Pipeline = raw.Pipeline
	c.RemoteCacheOptions = raw.RemoteCacheOptions

	return nil
}
