package fs

import (
	"encoding/json"
	"fmt"
	"io/ioutil"
	"log"
	"os"
	"path/filepath"
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
	// Global root filesystem dependencies
	GlobalDependencies []string `json:"globalDependencies,omitempty"`
	// Global env
	GlobalEnv []string `json:"globalEnv,omitempty"`

	// Global passthrough env
	GlobalPassthroughEnv []string `json:"experimentalGlobalPassThroughEnv,omitempty"`

	// Pipeline is a map of Turbo pipeline entries which define the task graph
	// and cache behavior on a per task or per package-task basis.
	Pipeline Pipeline `json:"pipeline"`
	// Configuration options when interfacing with the remote cache
	RemoteCacheOptions RemoteCacheOptions `json:"remoteCache,omitempty"`

	// Extends can be the name of another workspace
	Extends []string `json:"extends,omitempty"`
}

// pristineTurboJSON is used when marshaling a TurboJSON object into a turbo.json string
// Notably, it includes a PristinePipeline instead of the regular Pipeline. (i.e. TaskDefinition
// instead of BookkeepingTaskDefinition.)
type pristineTurboJSON struct {
	GlobalDependencies   []string           `json:"globalDependencies,omitempty"`
	GlobalEnv            []string           `json:"globalEnv,omitempty"`
	GlobalPassthroughEnv []string           `json:"experimentalGlobalPassThroughEnv,omitempty"`
	Pipeline             PristinePipeline   `json:"pipeline"`
	RemoteCacheOptions   RemoteCacheOptions `json:"remoteCache,omitempty"`
	Extends              []string           `json:"extends,omitempty"`
}

// TurboJSON represents a turbo.json configuration file
type TurboJSON struct {
	GlobalDeps           []string
	GlobalEnv            []string
	GlobalPassthroughEnv []string
	Pipeline             Pipeline
	RemoteCacheOptions   RemoteCacheOptions

	// A list of Workspace names
	Extends []string
}

// RemoteCacheOptions is a struct for deserializing .remoteCache of configFile
type RemoteCacheOptions struct {
	TeamID    string `json:"teamId,omitempty"`
	Signature bool   `json:"signature,omitempty"`
}

// rawTaskWithDefaults exists to Marshal (i.e. turn a TaskDefinition into json).
// We use this for printing ResolvedTaskConfiguration, because we _want_ to show
// the user the default values for key they have not configured.
type rawTaskWithDefaults struct {
	Outputs        []string            `json:"outputs"`
	Cache          *bool               `json:"cache"`
	DependsOn      []string            `json:"dependsOn"`
	Inputs         []string            `json:"inputs"`
	OutputMode     util.TaskOutputMode `json:"outputMode"`
	PassthroughEnv []string            `json:"experimentalPassThroughEnv,omitempty"`
	Env            []string            `json:"env"`
	Persistent     bool                `json:"persistent"`
}

// rawTask exists to Unmarshal from json. When fields are omitted, we _want_
// them to be missing, so that we can distinguish missing from empty value.
type rawTask struct {
	Outputs        []string             `json:"outputs,omitempty"`
	Cache          *bool                `json:"cache,omitempty"`
	DependsOn      []string             `json:"dependsOn,omitempty"`
	Inputs         []string             `json:"inputs,omitempty"`
	OutputMode     *util.TaskOutputMode `json:"outputMode,omitempty"`
	Env            []string             `json:"env,omitempty"`
	PassthroughEnv []string             `json:"experimentalPassthroughEnv,omitempty"`
	Persistent     *bool                `json:"persistent,omitempty"`
}

// taskDefinitionHashable exists as a definition for PristinePipeline, which is used down
// stream for calculating the global hash. We want to exclude experimental fields here
// because we don't want experimental fields to be part of the global hash.
type taskDefinitionHashable struct {
	Outputs                 TaskOutputs
	ShouldCache             bool
	EnvVarDependencies      []string
	TopologicalDependencies []string
	TaskDependencies        []string
	Inputs                  []string
	OutputMode              util.TaskOutputMode
	Persistent              bool
}

// taskDefinitionExperiments is a list of config fields in a task definition that are considered
// experimental. We keep these separated so we can compute a global hash without these.
type taskDefinitionExperiments struct {
	PassthroughEnv []string
}

// PristinePipeline is a map of task names to TaskDefinition or taskDefinitionHashable.
// Depending on whether any experimental fields are defined, we will use either struct.
// The purpose is to omit experimental fields when making a pristine version, so that
// it doesn't show up in --dry/--summarize output or affect the global hash.
type PristinePipeline map[string]interface{}

// Pipeline is a struct for deserializing .pipeline in configFile
type Pipeline map[string]BookkeepingTaskDefinition

// BookkeepingTaskDefinition holds the underlying TaskDefinition and some bookkeeping data
// about the TaskDefinition. This wrapper struct allows us to leave TaskDefinition untouched.
type BookkeepingTaskDefinition struct {
	definedFields      util.Set
	experimentalFields util.Set
	experimental       taskDefinitionExperiments
	TaskDefinition     taskDefinitionHashable
}

// TaskDefinition is a representation of the configFile pipeline for further computation.
type TaskDefinition struct {
	Outputs     TaskOutputs
	ShouldCache bool

	// This field is custom-marshalled from rawTask.Env and rawTask.DependsOn
	EnvVarDependencies []string

	// rawTask.PassthroughEnv
	PassthroughEnv []string

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
	turboFromFiles, err := readTurboConfig(dir.UntypedJoin(configFile))

	if !includeSynthesizedFromRootPackageJSON && err != nil {
		// If the file didn't exist, throw a custom error here instead of propagating
		if errors.Is(err, os.ErrNotExist) {
			return nil, errors.Wrap(err, fmt.Sprintf("Could not find %s. Follow directions at https://turbo.build/repo/docs to create one", configFile))

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
			// Explicitly set ShouldCache to false in this definition and add the bookkeeping fields
			// so downstream we can pretend that it was set on purpose (as if read from a config file)
			// rather than defaulting to the 0-value of a boolean field.
			turboJSON.Pipeline[taskName] = BookkeepingTaskDefinition{
				definedFields: util.SetFromStrings([]string{"ShouldCache"}),
				TaskDefinition: taskDefinitionHashable{
					ShouldCache: false,
				},
			}
		}
	}
	return turboJSON, nil
}

// TurboJSONValidation is the signature for a validation function passed to Validate()
type TurboJSONValidation func(*TurboJSON) []error

// Validate calls an array of validation functions on the TurboJSON struct.
// The validations can be customized by the caller.
func (tj *TurboJSON) Validate(validations []TurboJSONValidation) []error {
	allErrors := []error{}
	for _, validation := range validations {
		errors := validation(tj)
		allErrors = append(allErrors, errors...)
	}

	return allErrors
}

// TaskOutputs represents the patterns for including and excluding files from outputs
type TaskOutputs struct {
	Inclusions []string
	Exclusions []string
}

// Sort contents of task outputs
func (to TaskOutputs) Sort() TaskOutputs {
	var inclusions []string
	var exclusions []string
	copy(inclusions, to.Inclusions)
	copy(exclusions, to.Exclusions)
	sort.Strings(inclusions)
	sort.Strings(exclusions)
	return TaskOutputs{Inclusions: inclusions, Exclusions: exclusions}
}

// readTurboConfig reads turbo.json from a provided path
func readTurboConfig(turboJSONPath turbopath.AbsoluteSystemPath) (*TurboJSON, error) {
	// If the configFile exists, use that
	if turboJSONPath.FileExists() {
		turboJSON, err := readTurboJSON(turboJSONPath)
		if err != nil {
			return nil, fmt.Errorf("%s: %w", configFile, err)
		}

		return turboJSON, nil
	}

	// If there's no turbo.json, return an error.
	return nil, os.ErrNotExist
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
		return entry.GetTaskDefinition(), true
	}
	_, task := util.GetPackageTaskFromId(taskID)
	entry, ok := pc[task]
	return entry.GetTaskDefinition(), ok
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

// Pristine returns a PristinePipeline, this is used for printing to console and pruning
func (pc Pipeline) Pristine() PristinePipeline {
	pristine := PristinePipeline{}
	for taskName, taskDef := range pc {
		// If there are any experimental fields, we will include them with 0-values
		// if there aren't, we will omit them entirely
		if taskDef.hasExperimentalFields() {
			pristine[taskName] = taskDef.GetTaskDefinition() // merges experimental fields in
		} else {
			pristine[taskName] = taskDef.TaskDefinition // has no experimental fields
		}
	}
	return pristine
}

// hasField checks the internal bookkeeping definedFields field to
// see whether a field was actually in the underlying turbo.json
// or whether it was initialized with its 0-value.
func (btd BookkeepingTaskDefinition) hasField(fieldName string) bool {
	return btd.definedFields.Includes(fieldName) || btd.experimentalFields.Includes(fieldName)
}

// hasExperimentalFields keeps track of whether any experimental fields were found
func (btd BookkeepingTaskDefinition) hasExperimentalFields() bool {
	return len(btd.experimentalFields) > 0
}

// GetTaskDefinition gets a TaskDefinition by merging the experimental and non-experimental fields
// into a single representation to use downstream.
func (btd BookkeepingTaskDefinition) GetTaskDefinition() TaskDefinition {
	return TaskDefinition{
		Outputs:                 btd.TaskDefinition.Outputs,
		ShouldCache:             btd.TaskDefinition.ShouldCache,
		EnvVarDependencies:      btd.TaskDefinition.EnvVarDependencies,
		TopologicalDependencies: btd.TaskDefinition.TopologicalDependencies,
		TaskDependencies:        btd.TaskDefinition.TaskDependencies,
		Inputs:                  btd.TaskDefinition.Inputs,
		OutputMode:              btd.TaskDefinition.OutputMode,
		Persistent:              btd.TaskDefinition.Persistent,
		// From experimental fields
		PassthroughEnv: btd.experimental.PassthroughEnv,
	}
}

// MergeTaskDefinitions accepts an array of BookkeepingTaskDefinitions and merges them into
// a single TaskDefinition. It uses the bookkeeping definedFields to determine which fields should
// be overwritten and when 0-values should be respected.
func MergeTaskDefinitions(taskDefinitions []BookkeepingTaskDefinition) (*TaskDefinition, error) {
	// Start with an empty definition
	mergedTaskDefinition := &TaskDefinition{}

	// Set the default, because the 0-value will be false, and if no turbo.jsons had
	// this field set for this task, we want it to be true.
	mergedTaskDefinition.ShouldCache = true

	// For each of the TaskDefinitions we know of, merge them in
	for _, bookkeepingTaskDef := range taskDefinitions {
		taskDef := bookkeepingTaskDef.GetTaskDefinition()

		if bookkeepingTaskDef.hasField("Outputs") {
			mergedTaskDefinition.Outputs = taskDef.Outputs
		}

		if bookkeepingTaskDef.hasField("ShouldCache") {
			mergedTaskDefinition.ShouldCache = taskDef.ShouldCache
		}

		if bookkeepingTaskDef.hasField("EnvVarDependencies") {
			mergedTaskDefinition.EnvVarDependencies = taskDef.EnvVarDependencies
		}

		if bookkeepingTaskDef.hasField("PassthroughEnv") {
			mergedTaskDefinition.PassthroughEnv = taskDef.PassthroughEnv
		}

		if bookkeepingTaskDef.hasField("DependsOn") {
			mergedTaskDefinition.TopologicalDependencies = taskDef.TopologicalDependencies
		}

		if bookkeepingTaskDef.hasField("DependsOn") {
			mergedTaskDefinition.TaskDependencies = taskDef.TaskDependencies
		}

		if bookkeepingTaskDef.hasField("Inputs") {
			mergedTaskDefinition.Inputs = taskDef.Inputs
		}

		if bookkeepingTaskDef.hasField("OutputMode") {
			mergedTaskDefinition.OutputMode = taskDef.OutputMode
		}
		if bookkeepingTaskDef.hasField("Persistent") {
			mergedTaskDefinition.Persistent = taskDef.Persistent
		}
	}

	return mergedTaskDefinition, nil
}

// UnmarshalJSON deserializes a single task definition from
// turbo.json into a TaskDefinition struct
func (btd *BookkeepingTaskDefinition) UnmarshalJSON(data []byte) error {
	task := rawTask{}
	if err := json.Unmarshal(data, &task); err != nil {
		return err
	}

	btd.definedFields = util.Set{}
	btd.experimentalFields = util.Set{}

	if task.Outputs != nil {
		var inclusions []string
		var exclusions []string
		// Assign a bookkeeping field so we know that there really were
		// outputs configured in the underlying config file.
		btd.definedFields.Add("Outputs")

		for _, glob := range task.Outputs {
			if strings.HasPrefix(glob, "!") {
				if filepath.IsAbs(glob[1:]) {
					log.Printf("[WARNING] Using an absolute path in \"outputs\" (%v) will not work and will be an error in a future version", glob)
				}
				exclusions = append(exclusions, glob[1:])
			} else {
				if filepath.IsAbs(glob) {
					log.Printf("[WARNING] Using an absolute path in \"outputs\" (%v) will not work and will be an error in a future version", glob)
				}
				inclusions = append(inclusions, glob)
			}
		}

		btd.TaskDefinition.Outputs = TaskOutputs{
			Inclusions: inclusions,
			Exclusions: exclusions,
		}

		sort.Strings(btd.TaskDefinition.Outputs.Inclusions)
		sort.Strings(btd.TaskDefinition.Outputs.Exclusions)
	}

	if task.Cache == nil {
		btd.TaskDefinition.ShouldCache = true
	} else {
		btd.definedFields.Add("ShouldCache")
		btd.TaskDefinition.ShouldCache = *task.Cache
	}

	envVarDependencies := make(util.Set)
	envVarPassthroughs := make(util.Set)

	btd.TaskDefinition.TopologicalDependencies = []string{} // TODO @mehulkar: this should be a set
	btd.TaskDefinition.TaskDependencies = []string{}        // TODO @mehulkar: this should be a set

	// If there was a dependsOn field, add the bookkeeping
	// we don't care what's in the field, just that it was there
	// We'll use this marker to overwrite while merging TaskDefinitions.
	if task.DependsOn != nil {
		btd.definedFields.Add("DependsOn")
	}

	for _, dependency := range task.DependsOn {
		if strings.HasPrefix(dependency, envPipelineDelimiter) {
			log.Printf("[DEPRECATED] Declaring an environment variable in \"dependsOn\" is deprecated, found %s. Use the \"env\" key or use `npx @turbo/codemod migrate-env-var-dependencies`.\n", dependency)
			envVarDependencies.Add(strings.TrimPrefix(dependency, envPipelineDelimiter))
		} else if strings.HasPrefix(dependency, topologicalPipelineDelimiter) {
			// Note: This will get assigned multiple times in the loop, but we only care that it's true
			btd.TaskDefinition.TopologicalDependencies = append(btd.TaskDefinition.TopologicalDependencies, strings.TrimPrefix(dependency, topologicalPipelineDelimiter))
		} else {
			btd.TaskDefinition.TaskDependencies = append(btd.TaskDefinition.TaskDependencies, dependency)
		}
	}

	sort.Strings(btd.TaskDefinition.TaskDependencies)
	sort.Strings(btd.TaskDefinition.TopologicalDependencies)

	// Append env key into EnvVarDependencies
	if task.Env != nil {
		btd.definedFields.Add("EnvVarDependencies")
		if err := gatherEnvVars(task.Env, "env", &envVarDependencies); err != nil {
			return err
		}
	}

	btd.TaskDefinition.EnvVarDependencies = envVarDependencies.UnsafeListOfStrings()

	sort.Strings(btd.TaskDefinition.EnvVarDependencies)

	if task.PassthroughEnv != nil {
		btd.experimentalFields.Add("PassthroughEnv")
		if err := gatherEnvVars(task.PassthroughEnv, "passthrougEnv", &envVarPassthroughs); err != nil {
			return err
		}
	}

	btd.experimental.PassthroughEnv = envVarPassthroughs.UnsafeListOfStrings()
	sort.Strings(btd.experimental.PassthroughEnv)

	if task.Inputs != nil {
		// Note that we don't require Inputs to be sorted, we're going to
		// hash the resulting files and sort that instead
		btd.definedFields.Add("Inputs")
		// TODO: during rust port, this should be moved to a post-parse validation step
		for _, input := range task.Inputs {
			if filepath.IsAbs(input) {
				log.Printf("[WARNING] Using an absolute path in \"inputs\" (%v) will not work and will be an error in a future version", input)
			}
		}
		btd.TaskDefinition.Inputs = task.Inputs
	}

	if task.OutputMode != nil {
		btd.definedFields.Add("OutputMode")
		btd.TaskDefinition.OutputMode = *task.OutputMode
	}

	if task.Persistent != nil {
		btd.definedFields.Add("Persistent")
		btd.TaskDefinition.Persistent = *task.Persistent
	} else {
		btd.TaskDefinition.Persistent = false
	}
	return nil
}

// MarshalJSON serializes taskDefinitionHashable struct into json
func (c taskDefinitionHashable) MarshalJSON() ([]byte, error) {
	task := makeRawTask(
		c.Persistent,
		c.ShouldCache,
		c.OutputMode,
		c.Inputs,
		c.Outputs,
		c.EnvVarDependencies,
		c.TaskDependencies,
		c.TopologicalDependencies,
	)
	return json.Marshal(task)
}

// MarshalJSON serializes TaskDefinition struct into json
func (c TaskDefinition) MarshalJSON() ([]byte, error) {
	task := makeRawTask(
		c.Persistent,
		c.ShouldCache,
		c.OutputMode,
		c.Inputs,
		c.Outputs,
		c.EnvVarDependencies,
		c.TaskDependencies,
		c.TopologicalDependencies,
	)

	if len(c.PassthroughEnv) > 0 {
		task.PassthroughEnv = append(task.PassthroughEnv, c.PassthroughEnv...)
	}
	sort.Strings(task.PassthroughEnv)

	return json.Marshal(task)
}

// UnmarshalJSON deserializes the contents of turbo.json into a TurboJSON struct
func (c *TurboJSON) UnmarshalJSON(data []byte) error {
	raw := &rawTurboJSON{}
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}

	envVarDependencies := make(util.Set)
	envVarPassthroughs := make(util.Set)
	globalFileDependencies := make(util.Set)

	if err := gatherEnvVars(raw.GlobalEnv, "globalEnv", &envVarDependencies); err != nil {
		return err
	}
	if err := gatherEnvVars(raw.GlobalPassthroughEnv, "experimentalGlobalPassThroughEnv", &envVarPassthroughs); err != nil {
		return err
	}

	// TODO: In the rust port, warnings should be refactored to a post-parse validation step
	for _, value := range raw.GlobalDependencies {
		if strings.HasPrefix(value, envPipelineDelimiter) {
			log.Printf("[DEPRECATED] Declaring an environment variable in \"globalDependencies\" is deprecated, found %s. Use the \"globalEnv\" key or use `npx @turbo/codemod migrate-env-var-dependencies`.\n", value)
			envVarDependencies.Add(strings.TrimPrefix(value, envPipelineDelimiter))
		} else {
			if filepath.IsAbs(value) {
				log.Printf("[WARNING] Using an absolute path in \"globalDependencies\" (%v) will not work and will be an error in a future version", value)
			}
			globalFileDependencies.Add(value)
		}
	}

	// turn the set into an array and assign to the TurboJSON struct fields.
	c.GlobalEnv = envVarDependencies.UnsafeListOfStrings()
	sort.Strings(c.GlobalEnv)

	if raw.GlobalPassthroughEnv != nil {
		c.GlobalPassthroughEnv = envVarPassthroughs.UnsafeListOfStrings()
		sort.Strings(c.GlobalPassthroughEnv)
	}

	c.GlobalDeps = globalFileDependencies.UnsafeListOfStrings()
	sort.Strings(c.GlobalDeps)

	// copy these over, we don't need any changes here.
	c.Pipeline = raw.Pipeline
	c.RemoteCacheOptions = raw.RemoteCacheOptions
	c.Extends = raw.Extends

	return nil
}

// MarshalJSON converts a TurboJSON into the equivalent json object in bytes
// note: we go via rawTurboJSON so that the output format is correct.
// This is used by `turbo prune` to generate a pruned turbo.json
// and also by --summarize & --dry=json to serialize the known config
// into something we can print to screen
func (c *TurboJSON) MarshalJSON() ([]byte, error) {
	raw := pristineTurboJSON{}
	raw.GlobalDependencies = c.GlobalDeps
	raw.GlobalEnv = c.GlobalEnv
	raw.GlobalPassthroughEnv = c.GlobalPassthroughEnv
	raw.Pipeline = c.Pipeline.Pristine()
	raw.RemoteCacheOptions = c.RemoteCacheOptions

	return json.Marshal(&raw)
}

func makeRawTask(persistent bool, shouldCache bool, outputMode util.TaskOutputMode, inputs []string, outputs TaskOutputs, envVarDependencies []string, taskDependencies []string, topologicalDependencies []string) *rawTaskWithDefaults {
	// Initialize with empty arrays, so we get empty arrays serialized into JSON
	task := &rawTaskWithDefaults{
		Outputs:        []string{},
		Inputs:         []string{},
		Env:            []string{},
		PassthroughEnv: []string{},
		DependsOn:      []string{},
	}

	task.Persistent = persistent
	task.Cache = &shouldCache
	task.OutputMode = outputMode

	if len(inputs) > 0 {
		task.Inputs = inputs
	}

	if len(envVarDependencies) > 0 {
		task.Env = append(task.Env, envVarDependencies...)
	}

	if len(outputs.Inclusions) > 0 {
		task.Outputs = append(task.Outputs, outputs.Inclusions...)
	}

	for _, i := range outputs.Exclusions {
		task.Outputs = append(task.Outputs, "!"+i)
	}

	if len(taskDependencies) > 0 {
		task.DependsOn = append(task.DependsOn, taskDependencies...)
	}

	for _, i := range topologicalDependencies {
		task.DependsOn = append(task.DependsOn, "^"+i)
	}

	// These _should_ already be sorted when the TaskDefinition struct was unmarshaled,
	// but we want to ensure they're sorted on the way out also, just in case something
	// in the middle mutates the items.
	sort.Strings(task.DependsOn)
	sort.Strings(task.Outputs)
	sort.Strings(task.Env)
	sort.Strings(task.Inputs)
	return task
}

// gatherEnvVars puts env vars into the provided set as long as they don't have an invalid value.
func gatherEnvVars(vars []string, key string, into *util.Set) error {
	for _, value := range vars {
		if strings.HasPrefix(value, envPipelineDelimiter) {
			// Hard error to help people specify this correctly during migration.
			// TODO: Remove this error after we have run summary.
			return fmt.Errorf("You specified \"%s\" in the \"%s\" key. You should not prefix your environment variables with \"%s\"", value, key, envPipelineDelimiter)
		}

		into.Add(value)
	}

	return nil
}
