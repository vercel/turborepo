package fs

import (
	"encoding/json"
	"fmt"
	"io/ioutil"
	"log"
	"os"
	"sort"
	"strings"

	"github.com/muhammadmuzzammil1998/jsonc"
	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/util"
)

const (
	configFile                   = "turbo.json"
	envPipelineDelimiter         = "$"
	topologicalPipelineDelimiter = "^"
)

type rawTurboJSON struct {
	GlobalDependencies []string           `json:"globalDependencies,omitempty"` // files that affect every task
	GlobalEnv          []string           `json:"globalEnv,omitempty"`          // env vars that affect every task
	Pipeline           Pipeline           `json:"pipeline"`                     // map of tasks that define the task graph and cache behavior on a per task or per package-task basis.
	RemoteCacheOptions RemoteCacheOptions `json:"remoteCache,omitempty"`        // options to interface with the remote cache
	Extends            []string           `json:"extends,omitempty"`            // to inherit config from another workspace
}

// rawTaskWithDefaults exists to Marshal (i.e. turn a TaskDefinition into json).
// We use this to print the ResolvedTaskConfiguration, because we want to show
// the user the default values for key they have not configured.
type rawTaskWithDefaults struct {
	Outputs    []string            `json:"outputs"`
	Cache      *bool               `json:"cache"`
	DependsOn  []string            `json:"dependsOn"`
	Inputs     []string            `json:"inputs"`
	OutputMode util.TaskOutputMode `json:"outputMode"`
	Env        []string            `json:"env"`
	Persistent bool                `json:"persistent"`
}

// rawTask exists to Unmarshal from json. When fields are omitted, we _want_
// them to be missing, so that we can distinguish missing from empty value.
type rawTask struct {
	Outputs    []string             `json:"outputs,omitempty"`
	Cache      *bool                `json:"cache,omitempty"`
	DependsOn  []string             `json:"dependsOn,omitempty"`
	Inputs     []string             `json:"inputs,omitempty"`
	OutputMode *util.TaskOutputMode `json:"outputMode,omitempty"`
	Env        []string             `json:"env,omitempty"`
	Persistent bool                 `json:"persistent,omitempty"`
}

// TurboJSON represents a turbo.json configuration file
type TurboJSON struct {
	GlobalDeps         []string
	GlobalEnv          []string
	Pipeline           Pipeline
	RemoteCacheOptions RemoteCacheOptions
	Extends            []string // A list of Workspace names
}

// RemoteCacheOptions is a struct for deserializing .remoteCache of configFile
type RemoteCacheOptions struct {
	TeamID    string `json:"teamId,omitempty"`
	Signature bool   `json:"signature,omitempty"`
}

// Pipeline is a struct for deserializing .pipeline in configFile
type Pipeline map[string]BookkeepingTaskDefinition

type BookkeepingTaskDefinition struct {
	fieldsMeta     map[string]bool // holds some bookkeeping info about the TaskDefinition
	TaskDefinition TaskDefinition
}

// TaskDefinition is a representation of the configFile pipeline for further computation.
type TaskDefinition struct {
	Outputs     TaskOutputs
	ShouldCache bool

	// This field is custom-marshalled from rawTask.Env and rawTask.DependsOn
	EnvVarDependencies []string

	// TopologicalDependencies are tasks from package dependencies.
	// E.g. "build" is a topological dependency in:
	// dependsOn: ['^build'].
	// This field is custom-marshalled from rawTask.DependsOn
	TopologicalDependencies []string

	// TaskDependencies are anything that is not a topological dependency
	// E.g. both something and //whatever are TaskDependencies in:
	// dependsOn: ['something', '//whatever']
	// This field is custom-marshalled from rawTask.DependsOn
	TaskDependencies []string

	// Inputs indicate the list of files this Task depends on. If any of those files change
	// we can conclude that any cached outputs or logs for this Task should be invalidated.
	Inputs []string

	// OutputMode determins how we should log the output.
	OutputMode util.TaskOutputMode

	// Persistent indicates whether the Task is expected to exit or not
	// Tasks marked Persistent do not exit (e.g. --watch mode or dev servers)
	Persistent bool
}

// ResolvedTaskDefinition is a modified version the TaskDefinition struct.
// It is used for merged task definitions, and for printing out in Run Summaries.
type ResolvedTaskDefinition struct {
	Outputs                 *TaskOutputs
	ShouldCache             bool
	EnvVarDependencies      []string
	TopologicalDependencies []string
	TaskDependencies        []string
	Inputs                  []string
	OutputMode              util.TaskOutputMode
	Persistent              bool
}

// GetTask returns a TaskDefinition based on the ID (package#task format) or name (e.g. "build")
func (pc Pipeline) GetTask(taskID string, taskName string) (*BookkeepingTaskDefinition, error) {
	// first check for package-tasks
	taskDefinition, ok := pc[taskID]
	if !ok {
		// then check for regular tasks
		fallbackTaskDefinition, notcool := pc[taskName]
		// if neither, then bail
		if !notcool {
			// Return an empty TaskDefinition
			return nil, fmt.Errorf("Could not find task \"%s\" in pipeline", taskID)
		}

		// override if we need to...
		taskDefinition = fallbackTaskDefinition
	}

	return &taskDefinition, nil
}

// LoadTurboConfig loads, or optionally, synthesizes a TurboJSON instance
func LoadTurboConfig(dir turbopath.AbsoluteSystemPath, rootPackageJSON *PackageJSON, includeSynthesizedFromRootPackageJSON bool) (*TurboJSON, error) {
	// If the root package.json stil has a `turbo` key, log a warning and remove it.
	if rootPackageJSON.LegacyTurboConfig != nil {
		log.Printf("[WARNING] \"turbo\" in package.json is no longer supported. Migrate to %s by running \"npx @turbo/codemod create-turbo-config\"\n", configFile)
		rootPackageJSON.LegacyTurboConfig = nil
	}

	var turboJSON *TurboJSON
	turboFromFiles, err := ReadTurboConfig(dir.UntypedJoin(configFile))

	if !includeSynthesizedFromRootPackageJSON && err != nil {
		// If the file didn't exist, throw a custom error here instead of propagating
		if errors.Is(err, os.ErrNotExist) {
			return nil, fmt.Errorf("Could not find %s. Follow directions at https://turbo.build/repo/docs to create one: file does not exist", configFile)
		}

		// There was an error, and we don't have any chance of recovering
		// because we aren't synthesizing anything
		return nil, err
	} else if !includeSynthesizedFromRootPackageJSON {
		// We're not synthesizing anything and there was no error, we're done
		return turboFromFiles, nil
	} else if errors.Is(err, os.ErrNotExist) {
		// turbo.json doesn't exist, but we're going try to synthesize something
		turboJSON = &TurboJSON{
			Pipeline: make(Pipeline),
		}
	} else if err != nil {
		// some other happened, we can't recover
		return nil, err
	} else {
		// we're synthesizing, but we have a starting point
		// Note: this will have to change to support task inference in a monorepo
		// for now, we're going to error on any "root" tasks and turn non-root tasks into root tasks
		pipeline := make(Pipeline)
		for taskID, taskDefinition := range turboFromFiles.Pipeline {
			if util.IsPackageTask(taskID) {
				return nil, fmt.Errorf("Package tasks (<package>#<task>) are not allowed in single-package repositories: found %v", taskID)
			}
			pipeline[util.RootTaskID(taskID)] = taskDefinition
		}
		turboJSON = turboFromFiles
		turboJSON.Pipeline = pipeline
	}

	for scriptName := range rootPackageJSON.Scripts {
		if !turboJSON.Pipeline.HasTask(scriptName) {
			taskName := util.RootTaskID(scriptName)
			turboJSON.Pipeline[taskName] = BookkeepingTaskDefinition{
				fieldsMeta:     map[string]bool{},
				TaskDefinition: TaskDefinition{},
			}
		}
	}
	return turboJSON, nil
}

// TaskOutputs represents the patterns for including and excluding files from outputs
type TaskOutputs struct {
	Inclusions []string
	Exclusions []string
}

// Sort contents of task outputs
func (to *TaskOutputs) Sort() *TaskOutputs {
	var inclusions []string
	var exclusions []string
	copy(inclusions, to.Inclusions)
	copy(exclusions, to.Exclusions)
	sort.Strings(inclusions)
	sort.Strings(exclusions)
	return &TaskOutputs{Inclusions: inclusions, Exclusions: exclusions}
}

// ReadTurboConfig reads turbo.json from a provided path
func ReadTurboConfig(turboJSONPath turbopath.AbsoluteSystemPath) (*TurboJSON, error) {
	// If the configFile exists, use that
	if turboJSONPath.FileExists() {
		turboJSON, err := readTurboJSON(turboJSONPath)
		if err != nil {
			return nil, fmt.Errorf("%s: %w", configFile, err)
		}

		return turboJSON, nil
	}

	// If there's no turbo.json, return an error.
	return nil, errors.Wrapf(os.ErrNotExist, "Could not find %s", configFile)
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

	fmt.Printf("[debug] turboJSON %#v\n", turboJSON)
	return turboJSON, nil
}

// GetTaskDefinition returns a TaskDefinition from a serialized definition in configFile
func (pc Pipeline) GetTaskDefinition(taskID string) (TaskDefinition, bool) {
	if entry, ok := pc[taskID]; ok {
		return entry.TaskDefinition, true
	}
	_, task := util.GetPackageTaskFromId(taskID)
	entry, ok := pc[task]
	return entry.TaskDefinition, ok
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

// HasField checks the internal bookkeeping fieldsMeta field to
// see whether a field was actually in the underlying turbo.json
// or whether it was initialized with its 0-value.
func (btd BookkeepingTaskDefinition) HasField(fieldName string) bool {
	if _, ok := btd.fieldsMeta[fieldName]; ok {
		return true
	}
	return false
}

// ----------- Unmarshaling from JSON ----------------- //

// UnmarshalJSON deserializes the contents of turbo.json into a TurboJSON struct
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
			log.Printf("[DEPRECATED] Declaring an environment variable in \"globalDependencies\" is deprecated, found %s. Use the \"globalEnv\" key or use `npx @turbo/codemod migrate-env-var-dependencies`.\n", value)
			envVarDependencies.Add(strings.TrimPrefix(value, envPipelineDelimiter))
		} else {
			globalFileDependencies.Add(value)
		}
	}

	// turn the set into an array and assign to the TurboJSON struct fields.
	c.GlobalEnv = envVarDependencies.UnsafeListOfStrings()
	sort.Strings(c.GlobalEnv)
	c.GlobalDeps = globalFileDependencies.UnsafeListOfStrings()
	sort.Strings(c.GlobalDeps)

	// copy these over, we don't need any changes here.
	c.Pipeline = raw.Pipeline
	c.RemoteCacheOptions = raw.RemoteCacheOptions
	c.Extends = raw.Extends

	return nil
}

// UnmarshalJSON deserializes a single task definition from
// turbo.json into a TaskDefinition struct
func (btd *BookkeepingTaskDefinition) UnmarshalJSON(data []byte) error {
	task := rawTask{}
	if err := json.Unmarshal(data, &task); err != nil {
		return err
	}

	btd.fieldsMeta = map[string]bool{}
	c := &btd.TaskDefinition

	fmt.Printf("[debug] task.Outputs %#v\n", task.Outputs)
	if task.Outputs != nil {
		var inclusions []string
		var exclusions []string
		// Assign a bookkeeping field so we know that there really were
		// outputs configured in the underlying config file.
		btd.fieldsMeta["Outputs"] = true

		for _, glob := range task.Outputs {
			if strings.HasPrefix(glob, "!") {
				exclusions = append(exclusions, glob[1:])
			} else {
				inclusions = append(inclusions, glob)
			}
		}

		c.Outputs = TaskOutputs{
			Inclusions: inclusions,
			Exclusions: exclusions,
		}

		sort.Strings(c.Outputs.Inclusions)
		sort.Strings(c.Outputs.Exclusions)
	}

	if task.Cache == nil {
		c.ShouldCache = true
	} else {
		c.ShouldCache = *task.Cache
	}

	envVarDependencies := make(util.Set)

	c.TopologicalDependencies = []string{} // TODO @mehulkar: this should be a set
	c.TaskDependencies = []string{}        // TODO @mehulkar: this should be a set

	if task.DependsOn != nil {
		for _, dependency := range task.DependsOn {
			if strings.HasPrefix(dependency, envPipelineDelimiter) {
				log.Printf("[DEPRECATED] Declaring an environment variable in \"dependsOn\" is deprecated, found %s. Use the \"env\" key or use `npx @turbo/codemod migrate-env-var-dependencies`.\n", dependency)
				envVarDependencies.Add(strings.TrimPrefix(dependency, envPipelineDelimiter))
			} else if strings.HasPrefix(dependency, topologicalPipelineDelimiter) {
				// Assign bookkeeping, but only once, since we are in a loop
				if _, ok := btd.fieldsMeta["TopologicalDependencies"]; !ok {
					btd.fieldsMeta["TopologicalDependencies"] = true
				}
				c.TopologicalDependencies = append(c.TopologicalDependencies, strings.TrimPrefix(dependency, topologicalPipelineDelimiter))
			} else {
				// Assign bookkeeping, but only once, since we are in a loop
				if _, ok := btd.fieldsMeta["TaskDependencies"]; !ok {
					btd.fieldsMeta["TaskDependencies"] = true
				}
				c.TaskDependencies = append(c.TaskDependencies, dependency)
			}
		}
	}

	sort.Strings(c.TaskDependencies)
	sort.Strings(c.TopologicalDependencies)

	// Append env key into EnvVarDependencies
	if task.Env != nil {
		btd.fieldsMeta["EnvVarDependencies"] = true
		for _, value := range task.Env {
			if strings.HasPrefix(value, envPipelineDelimiter) {
				// Hard error to help people specify this correctly during migration.
				// TODO: Remove this error after we have run summary.
				return fmt.Errorf("You specified \"%s\" in the \"env\" key. You should not prefix your environment variables with \"$\"", value)
			}

			envVarDependencies.Add(value)
		}
	}

	c.EnvVarDependencies = envVarDependencies.UnsafeListOfStrings()

	sort.Strings(c.EnvVarDependencies)

	if task.Inputs != nil {
		// Note that we don't require Inputs to be sorted, we're going to
		// hash the resulting files and sort that instead
		btd.fieldsMeta["Inputs"] = true
		c.Inputs = task.Inputs
	}

	if task.OutputMode != nil {
		btd.fieldsMeta["OutputMode"] = true
		c.OutputMode = *task.OutputMode
	}
	c.Persistent = task.Persistent
	return nil
}

// -------------- Marshalling into JSON -------------- //

// MarshalJSON converts a TurboJSON into the equivalent json object in bytes
// note: we go via rawTurboJSON so that the output format is correct
func (c *TurboJSON) MarshalJSON() ([]byte, error) {
	raw := rawTurboJSON{}
	raw.GlobalDependencies = c.GlobalDeps
	raw.GlobalEnv = c.GlobalEnv
	raw.Pipeline = c.Pipeline
	raw.RemoteCacheOptions = c.RemoteCacheOptions

	return json.Marshal(&raw)
}

// MarshalJSON serializes TaskDefinition struct into JSON.
func (c TaskDefinition) MarshalJSON() ([]byte, error) {
	// Initialize with empty arrays, so we get empty arrays serialized into JSON
	task := rawTaskWithDefaults{
		Outputs:   []string{},
		Inputs:    []string{},
		Env:       []string{},
		DependsOn: []string{},
	}

	task.Persistent = c.Persistent
	task.Cache = &c.ShouldCache
	task.OutputMode = c.OutputMode

	if len(c.Inputs) > 0 {
		task.Inputs = c.Inputs
	}

	if len(c.EnvVarDependencies) > 0 {
		task.Env = append(task.Env, c.EnvVarDependencies...)
	}

	if len(c.Outputs.Inclusions) > 0 {
		task.Outputs = append(task.Outputs, c.Outputs.Inclusions...)
	}

	for _, i := range c.Outputs.Exclusions {
		task.Outputs = append(task.Outputs, "!"+i)
	}

	if len(c.TaskDependencies) > 0 {
		task.DependsOn = append(task.DependsOn, c.TaskDependencies...)
	}

	for _, i := range c.TopologicalDependencies {
		task.DependsOn = append(task.DependsOn, "^"+i)
	}

	// These _should_ already be sorted when the TaskDefinition struct was unmarshaled,
	// but we want to ensure they're sorted on the way out also, just in case something
	// in the middle mutates the items.
	sort.Strings(task.DependsOn)
	sort.Strings(task.Outputs)
	sort.Strings(task.Env)
	sort.Strings(task.Inputs)

	return json.Marshal(task)
}

// MarshalJSON serializes ResolvedTaskDefinition struct into json
// It sets defaults for empty fields. This is used to print out a Run Summary
func (c *ResolvedTaskDefinition) MarshalJSON() ([]byte, error) {
	// Initialize with empty arrays, so we get empty arrays serialized into JSON
	task := rawTaskWithDefaults{
		Outputs:   []string{},
		Inputs:    []string{},
		Env:       []string{},
		DependsOn: []string{},
	}

	task.Persistent = c.Persistent
	task.Cache = &c.ShouldCache
	task.OutputMode = c.OutputMode

	if len(c.Inputs) > 0 {
		task.Inputs = c.Inputs
	}

	if len(c.EnvVarDependencies) > 0 {
		task.Env = append(task.Env, c.EnvVarDependencies...)
	}

	if len(c.Outputs.Inclusions) > 0 {
		task.Outputs = append(task.Outputs, c.Outputs.Inclusions...)
	}

	for _, i := range c.Outputs.Exclusions {
		task.Outputs = append(task.Outputs, "!"+i)
	}

	if len(c.TaskDependencies) > 0 {
		task.DependsOn = append(task.DependsOn, c.TaskDependencies...)
	}

	for _, i := range c.TopologicalDependencies {
		task.DependsOn = append(task.DependsOn, "^"+i)
	}

	// These _should_ already be sorted when the TaskDefinition struct was unmarshaled,
	// but we want to ensure they're sorted on the way out also, just in case something
	// in the middle mutates the items.
	sort.Strings(task.DependsOn)
	sort.Strings(task.Outputs)
	sort.Strings(task.Env)
	sort.Strings(task.Inputs)

	return json.Marshal(task)
}
