package fs

import (
	"encoding/json"
	"strings"

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
	Pipeline Pipeline
	// Configuration options when interfacing with the remote cache
	RemoteCacheOptions RemoteCacheOptions `json:"remoteCache,omitempty"`
}

func ReadTurboConfigJSON(path AbsolutePath) (*TurboConfigJSON, error) {
	file, err := path.Open()
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

type pipelineJSON struct {
	Outputs   *[]string `json:"outputs"`
	Cache     *bool     `json:"cache,omitempty"`
	DependsOn []string  `json:"dependsOn,omitempty"`
	Inputs    []string  `json:"inputs,omitempty"`
}

type Pipeline map[string]TaskDefinition

func (pc Pipeline) GetTaskDefinition(taskID string) (TaskDefinition, bool) {
	if entry, ok := pc[taskID]; ok {
		return entry, true
	}
	_, task := util.GetPackageTaskFromId(taskID)
	entry, ok := pc[task]
	return entry, ok
}

type TaskDefinition struct {
	Outputs                 []string
	ShouldCache             bool
	EnvVarDependencies      []string
	TopologicalDependencies []string
	TaskDependencies        []string
	Inputs                  []string
}

const (
	envPipelineDelimiter         = "$"
	topologicalPipelineDelimiter = "^"
)

var defaultOutputs = []string{"dist/**/*", "build/**/*"}

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
	return nil
}
