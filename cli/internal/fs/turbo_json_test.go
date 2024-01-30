package fs

import (
	"os"
	"reflect"
	"sort"
	"strings"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/vercel/turbo/cli/internal/fs/hash"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/util"
	"gotest.tools/v3/assert/cmp"
)

func assertIsSorted(t *testing.T, arr []string, msg string) {
	t.Helper()
	if arr == nil {
		return
	}

	copied := make([]string, len(arr))
	copy(copied, arr)
	sort.Strings(copied)
	if !reflect.DeepEqual(arr, copied) {
		t.Errorf("Expected sorted, got %v: %v", arr, msg)
	}
}

func Test_ReadTurboConfigDotEnvUndefined(t *testing.T) {
	testDir := getTestDir(t, "dotenv-undefined")
	turboJSON, turboJSONReadErr := readTurboConfig(testDir.UntypedJoin("turbo.json"))
	if turboJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", turboJSONReadErr)
	}

	// Undefined is nil.
	var typedNil turbopath.AnchoredUnixPathArray

	assert.Equal(t, typedNil, turboJSON.GlobalDotEnv)

	pipelineExpected := Pipeline{
		"build": {
			definedFields:      util.SetFromStrings([]string{}),
			experimentalFields: util.SetFromStrings([]string{}),
			experimental:       taskDefinitionExperiments{},
			TaskDefinition: taskDefinitionHashable{
				Outputs:                 hash.TaskOutputs{},
				Cache:                   true,
				TopologicalDependencies: []string{},
				TaskDependencies:        []string{},
				OutputMode:              util.FullTaskOutput,
				Env:                     []string{},
			},
		},
	}

	assert.Equal(t, pipelineExpected, turboJSON.Pipeline)

	// Snapshot test of serialization.
	bytes, _ := turboJSON.MarshalJSON()
	assert.Equal(t, "{\"globalPassThroughEnv\":null,\"globalDotEnv\":null,\"pipeline\":{\"build\":{\"outputs\":[],\"cache\":true,\"dependsOn\":[],\"inputs\":[],\"outputMode\":\"full\",\"persistent\":false,\"env\":[],\"passThroughEnv\":null,\"dotEnv\":null}},\"remoteCache\":{\"enabled\":true}}", string(bytes))
}

func Test_ReadTurboConfigDotEnvNull(t *testing.T) {
	testDir := getTestDir(t, "dotenv-null")
	turboJSON, turboJSONReadErr := readTurboConfig(testDir.UntypedJoin("turbo.json"))
	if turboJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", turboJSONReadErr)
	}

	// Undefined is nil.
	var typedNil turbopath.AnchoredUnixPathArray

	assert.Equal(t, typedNil, turboJSON.GlobalDotEnv)

	pipelineExpected := Pipeline{
		"build": {
			definedFields:      util.SetFromStrings([]string{}),
			experimentalFields: util.SetFromStrings([]string{}),
			experimental:       taskDefinitionExperiments{},
			TaskDefinition: taskDefinitionHashable{
				Outputs:                 hash.TaskOutputs{},
				Cache:                   true,
				TopologicalDependencies: []string{},
				TaskDependencies:        []string{},
				OutputMode:              util.FullTaskOutput,
				Env:                     []string{},
			},
		},
	}

	assert.Equal(t, pipelineExpected, turboJSON.Pipeline)

	// Snapshot test of serialization.
	bytes, _ := turboJSON.MarshalJSON()
	assert.Equal(t, "{\"globalPassThroughEnv\":null,\"globalDotEnv\":null,\"pipeline\":{\"build\":{\"outputs\":[],\"cache\":true,\"dependsOn\":[],\"inputs\":[],\"outputMode\":\"full\",\"persistent\":false,\"env\":[],\"passThroughEnv\":null,\"dotEnv\":null}},\"remoteCache\":{\"enabled\":true}}", string(bytes))
}

func Test_ReadTurboConfigDotEnvEmpty(t *testing.T) {
	testDir := getTestDir(t, "dotenv-empty")
	turboJSON, turboJSONReadErr := readTurboConfig(testDir.UntypedJoin("turbo.json"))
	if turboJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", turboJSONReadErr)
	}

	assert.Equal(t, make(turbopath.AnchoredUnixPathArray, 0), turboJSON.GlobalDotEnv)

	pipelineExpected := Pipeline{
		"build": {
			definedFields:      util.SetFromStrings([]string{"DotEnv"}),
			experimentalFields: util.SetFromStrings([]string{}),
			experimental:       taskDefinitionExperiments{},
			TaskDefinition: taskDefinitionHashable{
				Outputs:                 hash.TaskOutputs{},
				Cache:                   true,
				TopologicalDependencies: []string{},
				TaskDependencies:        []string{},
				OutputMode:              util.FullTaskOutput,
				Env:                     []string{},
				DotEnv:                  make(turbopath.AnchoredUnixPathArray, 0),
			},
		},
	}

	assert.Equal(t, pipelineExpected, turboJSON.Pipeline)

	// Snapshot test of serialization.
	bytes, _ := turboJSON.MarshalJSON()
	assert.Equal(t, "{\"globalPassThroughEnv\":null,\"globalDotEnv\":[],\"pipeline\":{\"build\":{\"outputs\":[],\"cache\":true,\"dependsOn\":[],\"inputs\":[],\"outputMode\":\"full\",\"persistent\":false,\"env\":[],\"passThroughEnv\":null,\"dotEnv\":[]}},\"remoteCache\":{\"enabled\":true}}", string(bytes))
}

func Test_ReadTurboConfigDotEnvPopulated(t *testing.T) {
	testDir := getTestDir(t, "dotenv-populated")
	turboJSON, turboJSONReadErr := readTurboConfig(testDir.UntypedJoin("turbo.json"))
	if turboJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", turboJSONReadErr)
	}

	assert.Equal(t, turbopath.AnchoredUnixPathArray{"z", "y", "x"}, turboJSON.GlobalDotEnv)

	pipelineExpected := Pipeline{
		"build": {
			definedFields:      util.SetFromStrings([]string{"DotEnv"}),
			experimentalFields: util.SetFromStrings([]string{}),
			experimental:       taskDefinitionExperiments{},
			TaskDefinition: taskDefinitionHashable{
				Outputs:                 hash.TaskOutputs{},
				Cache:                   true,
				TopologicalDependencies: []string{},
				TaskDependencies:        []string{},
				OutputMode:              util.FullTaskOutput,
				Env:                     []string{},
				DotEnv:                  turbopath.AnchoredUnixPathArray{"3", "2", "1"},
			},
		},
	}

	assert.Equal(t, pipelineExpected, turboJSON.Pipeline)

	// Snapshot test of serialization.
	bytes, _ := turboJSON.MarshalJSON()
	assert.Equal(t, "{\"globalPassThroughEnv\":null,\"globalDotEnv\":[\"z\",\"y\",\"x\"],\"pipeline\":{\"build\":{\"outputs\":[],\"cache\":true,\"dependsOn\":[],\"inputs\":[],\"outputMode\":\"full\",\"persistent\":false,\"env\":[],\"passThroughEnv\":null,\"dotEnv\":[\"3\",\"2\",\"1\"]}},\"remoteCache\":{\"enabled\":true}}", string(bytes))
}

func Test_ReadTurboConfigPassThroughEnvUndefined(t *testing.T) {
	testDir := getTestDir(t, "passthrough-undefined")
	turboJSON, turboJSONReadErr := readTurboConfig(testDir.UntypedJoin("turbo.json"))
	if turboJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", turboJSONReadErr)
	}

	// Undefined is nil.
	var typedNil []string

	assert.Equal(t, typedNil, turboJSON.GlobalPassThroughEnv)

	pipelineExpected := Pipeline{
		"build": {
			definedFields:      util.SetFromStrings([]string{}),
			experimentalFields: util.SetFromStrings([]string{}),
			experimental:       taskDefinitionExperiments{},
			TaskDefinition: taskDefinitionHashable{
				Outputs:                 hash.TaskOutputs{},
				Cache:                   true,
				TopologicalDependencies: []string{},
				TaskDependencies:        []string{},
				OutputMode:              util.FullTaskOutput,
				Env:                     []string{},
				PassThroughEnv:          typedNil,
			},
		},
	}

	assert.Equal(t, pipelineExpected, turboJSON.Pipeline)

	// Snapshot test of serialization.
	bytes, _ := turboJSON.MarshalJSON()
	assert.Equal(t, "{\"globalPassThroughEnv\":null,\"globalDotEnv\":null,\"pipeline\":{\"build\":{\"outputs\":[],\"cache\":true,\"dependsOn\":[],\"inputs\":[],\"outputMode\":\"full\",\"persistent\":false,\"env\":[],\"passThroughEnv\":null,\"dotEnv\":null}},\"remoteCache\":{\"enabled\":true}}", string(bytes))
}

func Test_ReadTurboConfigPassThroughEnvNull(t *testing.T) {
	testDir := getTestDir(t, "passthrough-null")
	turboJSON, turboJSONReadErr := readTurboConfig(testDir.UntypedJoin("turbo.json"))
	if turboJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", turboJSONReadErr)
	}

	// Undefined is nil.
	var typedNil []string

	assert.Equal(t, typedNil, turboJSON.GlobalPassThroughEnv)

	pipelineExpected := Pipeline{
		"build": {
			definedFields:      util.SetFromStrings([]string{}),
			experimentalFields: util.SetFromStrings([]string{}),
			experimental:       taskDefinitionExperiments{},
			TaskDefinition: taskDefinitionHashable{
				Outputs:                 hash.TaskOutputs{},
				Cache:                   true,
				TopologicalDependencies: []string{},
				TaskDependencies:        []string{},
				OutputMode:              util.FullTaskOutput,
				Env:                     []string{},
				PassThroughEnv:          typedNil,
			},
		},
	}

	assert.Equal(t, pipelineExpected, turboJSON.Pipeline)

	// Snapshot test of serialization.
	bytes, _ := turboJSON.MarshalJSON()
	assert.Equal(t, "{\"globalPassThroughEnv\":null,\"globalDotEnv\":null,\"pipeline\":{\"build\":{\"outputs\":[],\"cache\":true,\"dependsOn\":[],\"inputs\":[],\"outputMode\":\"full\",\"persistent\":false,\"env\":[],\"passThroughEnv\":null,\"dotEnv\":null}},\"remoteCache\":{\"enabled\":true}}", string(bytes))
}

func Test_ReadTurboConfigPassThroughEnvEmpty(t *testing.T) {
	testDir := getTestDir(t, "passthrough-empty")
	turboJSON, turboJSONReadErr := readTurboConfig(testDir.UntypedJoin("turbo.json"))
	if turboJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", turboJSONReadErr)
	}

	assert.Equal(t, []string{}, turboJSON.GlobalPassThroughEnv)

	pipelineExpected := Pipeline{
		"build": {
			definedFields:      util.SetFromStrings([]string{"PassThroughEnv"}),
			experimentalFields: util.SetFromStrings([]string{}),
			experimental:       taskDefinitionExperiments{},
			TaskDefinition: taskDefinitionHashable{
				Outputs:                 hash.TaskOutputs{},
				Cache:                   true,
				TopologicalDependencies: []string{},
				TaskDependencies:        []string{},
				OutputMode:              util.FullTaskOutput,
				Env:                     []string{},
				PassThroughEnv:          []string{},
			},
		},
	}

	assert.Equal(t, pipelineExpected, turboJSON.Pipeline)

	// Snapshot test of serialization.
	bytes, _ := turboJSON.MarshalJSON()
	assert.Equal(t, "{\"globalPassThroughEnv\":[],\"globalDotEnv\":null,\"pipeline\":{\"build\":{\"outputs\":[],\"cache\":true,\"dependsOn\":[],\"inputs\":[],\"outputMode\":\"full\",\"persistent\":false,\"env\":[],\"passThroughEnv\":[],\"dotEnv\":null}},\"remoteCache\":{\"enabled\":true}}", string(bytes))
}

func Test_ReadTurboConfigPassThroughEnvPopulated(t *testing.T) {
	testDir := getTestDir(t, "passthrough-populated")
	turboJSON, turboJSONReadErr := readTurboConfig(testDir.UntypedJoin("turbo.json"))
	if turboJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", turboJSONReadErr)
	}

	assert.Equal(t, []string{"A", "B", "C"}, turboJSON.GlobalPassThroughEnv)

	pipelineExpected := Pipeline{
		"build": {
			definedFields:      util.SetFromStrings([]string{"PassThroughEnv"}),
			experimentalFields: util.SetFromStrings([]string{}),
			experimental:       taskDefinitionExperiments{},
			TaskDefinition: taskDefinitionHashable{
				Outputs:                 hash.TaskOutputs{},
				Cache:                   true,
				TopologicalDependencies: []string{},
				TaskDependencies:        []string{},
				OutputMode:              util.FullTaskOutput,
				Env:                     []string{},
				PassThroughEnv:          []string{"X", "Y", "Z"},
			},
		},
	}

	assert.Equal(t, pipelineExpected, turboJSON.Pipeline)

	// Snapshot test of serialization.
	bytes, _ := turboJSON.MarshalJSON()
	assert.Equal(t, "{\"globalPassThroughEnv\":[\"A\",\"B\",\"C\"],\"globalDotEnv\":null,\"pipeline\":{\"build\":{\"outputs\":[],\"cache\":true,\"dependsOn\":[],\"inputs\":[],\"outputMode\":\"full\",\"persistent\":false,\"env\":[],\"passThroughEnv\":[\"X\",\"Y\",\"Z\"],\"dotEnv\":null}},\"remoteCache\":{\"enabled\":true}}", string(bytes))
}

func Test_ReadTurboConfig(t *testing.T) {
	testDir := getTestDir(t, "correct")
	turboJSON, turboJSONReadErr := readTurboConfig(testDir.UntypedJoin("turbo.json"))

	if turboJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", turboJSONReadErr)
	}

	assert.EqualValues(t, []string{"AWS_SECRET_KEY"}, turboJSON.GlobalPassThroughEnv)

	pipelineExpected := map[string]BookkeepingTaskDefinition{
		"build": {
			definedFields:      util.SetFromStrings([]string{"Outputs", "OutputMode", "DependsOn", "PassThroughEnv"}),
			experimentalFields: util.SetFromStrings([]string{}),
			experimental:       taskDefinitionExperiments{},
			TaskDefinition: taskDefinitionHashable{
				Outputs:                 hash.TaskOutputs{Inclusions: []string{".next/**", "dist/**"}, Exclusions: []string{"dist/assets/**"}},
				Cache:                   true,
				TopologicalDependencies: []string{"build"},
				TaskDependencies:        []string{},
				OutputMode:              util.NewTaskOutput,
				Env:                     []string{},
				PassThroughEnv:          []string{"GITHUB_TOKEN"},
			},
		},
		"lint": {
			definedFields:      util.SetFromStrings([]string{"Outputs", "OutputMode", "Cache", "DependsOn", "Env"}),
			experimentalFields: util.SetFromStrings([]string{}),
			experimental:       taskDefinitionExperiments{},
			TaskDefinition: taskDefinitionHashable{
				Outputs:                 hash.TaskOutputs{},
				Cache:                   true,
				TopologicalDependencies: []string{},
				TaskDependencies:        []string{},
				OutputMode:              util.NewTaskOutput,
				Env:                     []string{"MY_VAR"},
				PassThroughEnv:          nil,
			},
		},
		"dev": {
			definedFields:      util.SetFromStrings([]string{"OutputMode", "Cache", "PassThroughEnv"}),
			experimentalFields: util.SetFromStrings([]string{}),
			experimental:       taskDefinitionExperiments{},
			TaskDefinition: taskDefinitionHashable{
				Outputs:                 hash.TaskOutputs{},
				Cache:                   false,
				TopologicalDependencies: []string{},
				TaskDependencies:        []string{},
				OutputMode:              util.FullTaskOutput,
				Env:                     []string{},
				PassThroughEnv:          []string{},
			},
		},
		"publish": {
			definedFields:      util.SetFromStrings([]string{"Inputs", "Outputs", "DependsOn", "Cache"}),
			experimentalFields: util.SetFromStrings([]string{}),
			experimental:       taskDefinitionExperiments{},
			TaskDefinition: taskDefinitionHashable{
				Outputs:                 hash.TaskOutputs{Inclusions: []string{"dist/**"}},
				Cache:                   false,
				TopologicalDependencies: []string{"build", "publish"},
				TaskDependencies:        []string{"admin#lint", "build"},
				Inputs:                  []string{"build/**/*"},
				OutputMode:              util.FullTaskOutput,
				Env:                     []string{},
				PassThroughEnv:          nil,
			},
		},
	}

	validateOutput(t, turboJSON, pipelineExpected)
	remoteCacheOptionsExpected := RemoteCacheOptions{"team_id", true, true}
	assert.EqualValues(t, remoteCacheOptionsExpected, turboJSON.RemoteCacheOptions)
}

func Test_LoadTurboConfig_Legacy(t *testing.T) {
	testDir := getTestDir(t, "legacy-only")
	packageJSONPath := testDir.UntypedJoin("package.json")
	rootPackageJSON, pkgJSONReadErr := ReadPackageJSON(packageJSONPath)

	if pkgJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", pkgJSONReadErr)
	}

	_, turboJSONReadErr := LoadTurboConfig(testDir, rootPackageJSON, false)
	expectedErrorMsg := "Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create one: file does not exist"
	assert.EqualErrorf(t, turboJSONReadErr, expectedErrorMsg, "Error should be: %v, got: %v", expectedErrorMsg, turboJSONReadErr)
}

func Test_LoadTurboConfig_BothCorrectAndLegacy(t *testing.T) {
	testDir := getTestDir(t, "both")

	packageJSONPath := testDir.UntypedJoin("package.json")
	rootPackageJSON, pkgJSONReadErr := ReadPackageJSON(packageJSONPath)

	if pkgJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", pkgJSONReadErr)
	}

	turboJSON, turboJSONReadErr := LoadTurboConfig(testDir, rootPackageJSON, false)

	if turboJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", turboJSONReadErr)
	}

	pipelineExpected := map[string]BookkeepingTaskDefinition{
		"build": {
			definedFields:      util.SetFromStrings([]string{"Outputs", "OutputMode", "DependsOn"}),
			experimentalFields: util.SetFromStrings([]string{}),
			experimental:       taskDefinitionExperiments{},
			TaskDefinition: taskDefinitionHashable{
				Outputs:                 hash.TaskOutputs{Inclusions: []string{".next/**", "dist/**"}, Exclusions: []string{"dist/assets/**"}},
				Cache:                   true,
				TopologicalDependencies: []string{"build"},
				TaskDependencies:        []string{},
				OutputMode:              util.NewTaskOutput,
				Env:                     []string{},
				PassThroughEnv:          nil,
			},
		},
	}

	validateOutput(t, turboJSON, pipelineExpected)

	remoteCacheOptionsExpected := RemoteCacheOptions{"team_id", true, true}
	assert.EqualValues(t, remoteCacheOptionsExpected, turboJSON.RemoteCacheOptions)
	assert.Equal(t, rootPackageJSON.LegacyTurboConfig == nil, true)
}

func Test_ReadTurboConfig_InvalidEnvDeclarations1(t *testing.T) {
	testDir := getTestDir(t, "invalid-env-1")
	_, turboJSONReadErr := readTurboConfig(testDir.UntypedJoin("turbo.json"))

	expectedErrorMsg := "turbo.json: You specified \"$A\" in the \"env\" key. You should not prefix your environment variables with \"$\""
	assert.EqualErrorf(t, turboJSONReadErr, expectedErrorMsg, "Error should be: %v, got: %v", expectedErrorMsg, turboJSONReadErr)
}

func Test_ReadTurboConfig_InvalidEnvDeclarations2(t *testing.T) {
	testDir := getTestDir(t, "invalid-env-2")
	_, turboJSONReadErr := readTurboConfig(testDir.UntypedJoin("turbo.json"))
	expectedErrorMsg := "turbo.json: You specified \"$A\" in the \"env\" key. You should not prefix your environment variables with \"$\""
	assert.EqualErrorf(t, turboJSONReadErr, expectedErrorMsg, "Error should be: %v, got: %v", expectedErrorMsg, turboJSONReadErr)
}

func Test_ReadTurboConfig_InvalidGlobalEnvDeclarations(t *testing.T) {
	testDir := getTestDir(t, "invalid-global-env")
	_, turboJSONReadErr := readTurboConfig(testDir.UntypedJoin("turbo.json"))
	expectedErrorMsg := "turbo.json: You specified \"$QUX\" in the \"globalEnv\" key. You should not prefix your environment variables with \"$\""
	assert.EqualErrorf(t, turboJSONReadErr, expectedErrorMsg, "Error should be: %v, got: %v", expectedErrorMsg, turboJSONReadErr)
}

func Test_ReadTurboConfig_EnvDeclarations(t *testing.T) {
	testDir := getTestDir(t, "legacy-env")
	turboJSON, turboJSONReadErr := readTurboConfig(testDir.UntypedJoin("turbo.json"))

	if turboJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", turboJSONReadErr)
	}

	pipeline := turboJSON.Pipeline
	assert.EqualValues(t, pipeline["task1"].TaskDefinition.Env, sortedArray([]string{"A"}))
	assert.EqualValues(t, pipeline["task2"].TaskDefinition.Env, sortedArray([]string{"A"}))
	assert.EqualValues(t, pipeline["task3"].TaskDefinition.Env, sortedArray([]string{"A"}))
	assert.EqualValues(t, pipeline["task4"].TaskDefinition.Env, sortedArray([]string{"A", "B"}))
	assert.EqualValues(t, pipeline["task6"].TaskDefinition.Env, sortedArray([]string{"A", "B", "C", "D", "E", "F"}))
	assert.EqualValues(t, pipeline["task7"].TaskDefinition.Env, sortedArray([]string{"A", "B", "C"}))
	assert.EqualValues(t, pipeline["task8"].TaskDefinition.Env, sortedArray([]string{"A", "B", "C"}))
	assert.EqualValues(t, pipeline["task9"].TaskDefinition.Env, sortedArray([]string{"A"}))
	assert.EqualValues(t, pipeline["task10"].TaskDefinition.Env, sortedArray([]string{"A"}))
	assert.EqualValues(t, pipeline["task11"].TaskDefinition.Env, sortedArray([]string{"A", "B"}))

	// check global env vars also
	assert.EqualValues(t, sortedArray([]string{"FOO", "BAR", "BAZ", "QUX"}), sortedArray(turboJSON.GlobalEnv))
	assert.EqualValues(t, sortedArray([]string{"somefile.txt"}), sortedArray(turboJSON.GlobalDeps))
}

func Test_TaskOutputsSort(t *testing.T) {
	inclusions := []string{"foo/**", "bar"}
	exclusions := []string{"special-file", ".hidden/**"}
	taskOutputs := hash.TaskOutputs{Inclusions: inclusions, Exclusions: exclusions}
	taskOutputs.Sort()
	assertIsSorted(t, taskOutputs.Inclusions, "Inclusions")
	assertIsSorted(t, taskOutputs.Exclusions, "Exclusions")

	assert.True(t, cmp.DeepEqual(taskOutputs, hash.TaskOutputs{Inclusions: []string{"bar", "foo/**"}, Exclusions: []string{".hidden/**", "special-file"}})().Success())
}

// Helpers
func validateOutput(t *testing.T, turboJSON *TurboJSON, expectedPipeline Pipeline) {
	t.Helper()
	assertIsSorted(t, turboJSON.GlobalDeps, "Global Deps")
	assertIsSorted(t, turboJSON.GlobalEnv, "Global Env")
	assertIsSorted(t, turboJSON.GlobalPassThroughEnv, "Global Pass Through Env")
	validatePipeline(t, turboJSON.Pipeline, expectedPipeline)
}

func validatePipeline(t *testing.T, actual Pipeline, expected Pipeline) {
	t.Helper()
	// check top level keys
	if len(actual) != len(expected) {
		expectedKeys := []string{}
		for k := range expected {
			expectedKeys = append(expectedKeys, k)
		}
		actualKeys := []string{}
		for k := range actual {
			actualKeys = append(actualKeys, k)
		}
		t.Errorf("pipeline tasks mismatch. got %v, want %v", strings.Join(actualKeys, ","), strings.Join(expectedKeys, ","))
	}

	// check individual task definitions
	for taskName, expectedTaskDefinition := range expected {
		bookkeepingTaskDef, ok := actual[taskName]
		if !ok {
			t.Errorf("missing expected task: %v", taskName)
		}
		actualTaskDefinition := bookkeepingTaskDef.GetTaskDefinition()
		assertIsSorted(t, actualTaskDefinition.Outputs.Inclusions, "Task output inclusions")
		assertIsSorted(t, actualTaskDefinition.Outputs.Exclusions, "Task output exclusions")
		assertIsSorted(t, actualTaskDefinition.Env, "Task env vars")
		assertIsSorted(t, actualTaskDefinition.PassThroughEnv, "Task passthrough env vars")
		assertIsSorted(t, actualTaskDefinition.TopologicalDependencies, "Topo deps")
		assertIsSorted(t, actualTaskDefinition.TaskDependencies, "Task deps")
		assert.EqualValuesf(t, expectedTaskDefinition, bookkeepingTaskDef, "task definition mismatch for %v", taskName)
	}
}

func getTestDir(t *testing.T, testName string) turbopath.AbsoluteSystemPath {
	defaultCwd, err := os.Getwd()
	if err != nil {
		t.Errorf("failed to get cwd: %v", err)
	}
	cwd, err := CheckedToAbsoluteSystemPath(defaultCwd)
	if err != nil {
		t.Fatalf("cwd is not an absolute directory %v: %v", defaultCwd, err)
	}

	return cwd.UntypedJoin("testdata", testName)
}

func sortedArray(arr []string) []string {
	sort.Strings(arr)
	return arr
}
