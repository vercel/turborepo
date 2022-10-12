package fs

import (
	"os"
	"reflect"
	"sort"
	"strings"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/vercel/turborepo/cli/internal/turbopath"
	"github.com/vercel/turborepo/cli/internal/util"
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

func Test_ReadTurboConfig(t *testing.T) {
	testDir := getTestDir(t, "correct")

	packageJSONPath := testDir.UntypedJoin("package.json")
	rootPackageJSON, pkgJSONReadErr := ReadPackageJSON(packageJSONPath)

	if pkgJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", pkgJSONReadErr)
	}

	turboJSON, turboJSONReadErr := ReadTurboConfig(testDir, rootPackageJSON)

	if turboJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", turboJSONReadErr)
	}

	pipelineExpected := map[string]TaskDefinition{
		"build": {
			Outputs:                 TaskOutputs{Inclusions: []string{".next/**", "dist/**"}, Exclusions: []string{"dist/assets/**"}},
			TopologicalDependencies: []string{"build"},
			EnvVarDependencies:      []string{},
			TaskDependencies:        []string{},
			ShouldCache:             true,
			OutputMode:              util.NewTaskOutput,
		},
		"lint": {
			Outputs:                 TaskOutputs{},
			TopologicalDependencies: []string{},
			EnvVarDependencies:      []string{"MY_VAR"},
			TaskDependencies:        []string{},
			ShouldCache:             true,
			OutputMode:              util.NewTaskOutput,
		},
		"dev": {
			Outputs:                 defaultOutputs,
			TopologicalDependencies: []string{},
			EnvVarDependencies:      []string{},
			TaskDependencies:        []string{},
			ShouldCache:             false,
			OutputMode:              util.FullTaskOutput,
		},
		"publish": {
			Outputs:                 TaskOutputs{Inclusions: []string{"dist/**"}},
			TopologicalDependencies: []string{"build", "publish"},
			EnvVarDependencies:      []string{},
			TaskDependencies:        []string{"admin#lint", "build"},
			ShouldCache:             false,
			Inputs:                  []string{"build/**/*"},
			OutputMode:              util.FullTaskOutput,
		},
	}

	validateOutput(t, turboJSON, pipelineExpected)

	remoteCacheOptionsExpected := RemoteCacheOptions{"team_id", true}
	assert.EqualValues(t, remoteCacheOptionsExpected, turboJSON.RemoteCacheOptions)
}

func Test_ReadTurboConfig_Legacy(t *testing.T) {
	testDir := getTestDir(t, "legacy-only")

	packageJSONPath := testDir.UntypedJoin("package.json")
	rootPackageJSON, pkgJSONReadErr := ReadPackageJSON(packageJSONPath)

	if pkgJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", pkgJSONReadErr)
	}

	turboJSON, turboJSONReadErr := ReadTurboConfig(testDir, rootPackageJSON)

	if turboJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", turboJSONReadErr)
	}

	pipelineExpected := map[string]TaskDefinition{
		"build": {
			Outputs:                 TaskOutputs{Inclusions: []string{"build/**/*", "dist/**/*"}},
			TopologicalDependencies: []string{},
			EnvVarDependencies:      []string{},
			TaskDependencies:        []string{},
			ShouldCache:             true,
			OutputMode:              util.FullTaskOutput,
		},
	}

	validateOutput(t, turboJSON, pipelineExpected)
	assert.Empty(t, turboJSON.RemoteCacheOptions)
}

func Test_ReadTurboConfig_BothCorrectAndLegacy(t *testing.T) {
	testDir := getTestDir(t, "both")

	packageJSONPath := testDir.UntypedJoin("package.json")
	rootPackageJSON, pkgJSONReadErr := ReadPackageJSON(packageJSONPath)

	if pkgJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", pkgJSONReadErr)
	}

	turboJSON, turboJSONReadErr := ReadTurboConfig(testDir, rootPackageJSON)

	if turboJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", turboJSONReadErr)
	}

	pipelineExpected := map[string]TaskDefinition{
		"build": {
			Outputs:                 TaskOutputs{Inclusions: []string{".next/**", "dist/**"}, Exclusions: []string{"dist/assets/**"}},
			TopologicalDependencies: []string{"build"},
			EnvVarDependencies:      []string{},
			TaskDependencies:        []string{},
			ShouldCache:             true,
			OutputMode:              util.NewTaskOutput,
		},
	}

	validateOutput(t, turboJSON, pipelineExpected)

	remoteCacheOptionsExpected := RemoteCacheOptions{"team_id", true}
	assert.EqualValues(t, remoteCacheOptionsExpected, turboJSON.RemoteCacheOptions)

	assert.Equal(t, rootPackageJSON.LegacyTurboConfig == nil, true)
}

func Test_ReadTurboConfig_InvalidEnvDeclarations1(t *testing.T) {
	testDir := getTestDir(t, "invalid-env-1")

	packageJSONPath := testDir.UntypedJoin("package.json")
	rootPackageJSON, pkgJSONReadErr := ReadPackageJSON(packageJSONPath)

	if pkgJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", pkgJSONReadErr)
	}

	_, turboJSONReadErr := ReadTurboConfig(testDir, rootPackageJSON)

	expectedErrorMsg := "turbo.json: You specified \"$A\" in the \"env\" key. You should not prefix your environment variables with \"$\""

	assert.EqualErrorf(t, turboJSONReadErr, expectedErrorMsg, "Error should be: %v, got: %v", expectedErrorMsg, turboJSONReadErr)
}

func Test_ReadTurboConfig_InvalidEnvDeclarations2(t *testing.T) {
	testDir := getTestDir(t, "invalid-env-2")

	packageJSONPath := testDir.UntypedJoin("package.json")
	rootPackageJSON, pkgJSONReadErr := ReadPackageJSON(packageJSONPath)

	if pkgJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", pkgJSONReadErr)
	}

	_, turboJSONReadErr := ReadTurboConfig(testDir, rootPackageJSON)

	expectedErrorMsg := "turbo.json: You specified \"$A\" in the \"env\" key. You should not prefix your environment variables with \"$\""

	assert.EqualErrorf(t, turboJSONReadErr, expectedErrorMsg, "Error should be: %v, got: %v", expectedErrorMsg, turboJSONReadErr)
}

func Test_ReadTurboConfig_InvalidGlobalEnvDeclarations(t *testing.T) {
	testDir := getTestDir(t, "invalid-global-env")

	packageJSONPath := testDir.UntypedJoin("package.json")
	rootPackageJSON, pkgJSONReadErr := ReadPackageJSON(packageJSONPath)

	if pkgJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", pkgJSONReadErr)
	}

	_, turboJSONReadErr := ReadTurboConfig(testDir, rootPackageJSON)

	expectedErrorMsg := "turbo.json: You specified \"$QUX\" in the \"env\" key. You should not prefix your environment variables with \"$\""

	assert.EqualErrorf(t, turboJSONReadErr, expectedErrorMsg, "Error should be: %v, got: %v", expectedErrorMsg, turboJSONReadErr)
}

func Test_ReadTurboConfig_EnvDeclarations(t *testing.T) {
	testDir := getTestDir(t, "legacy-env")

	packageJSONPath := testDir.UntypedJoin("package.json")
	rootPackageJSON, pkgJSONReadErr := ReadPackageJSON(packageJSONPath)

	if pkgJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", pkgJSONReadErr)
	}

	turboJSON, turboJSONReadErr := ReadTurboConfig(testDir, rootPackageJSON)

	if turboJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", turboJSONReadErr)
	}

	pipeline := turboJSON.Pipeline
	assert.EqualValues(t, pipeline["task1"].EnvVarDependencies, sortedArray([]string{"A"}))
	assert.EqualValues(t, pipeline["task2"].EnvVarDependencies, sortedArray([]string{"A"}))
	assert.EqualValues(t, pipeline["task3"].EnvVarDependencies, sortedArray([]string{"A"}))
	assert.EqualValues(t, pipeline["task4"].EnvVarDependencies, sortedArray([]string{"A", "B"}))
	assert.EqualValues(t, pipeline["task6"].EnvVarDependencies, sortedArray([]string{"A", "B", "C", "D", "E", "F"}))
	assert.EqualValues(t, pipeline["task7"].EnvVarDependencies, sortedArray([]string{"A", "B", "C"}))
	assert.EqualValues(t, pipeline["task8"].EnvVarDependencies, sortedArray([]string{"A", "B", "C"}))
	assert.EqualValues(t, pipeline["task9"].EnvVarDependencies, sortedArray([]string{"A"}))
	assert.EqualValues(t, pipeline["task10"].EnvVarDependencies, sortedArray([]string{"A"}))
	assert.EqualValues(t, pipeline["task11"].EnvVarDependencies, sortedArray([]string{"A", "B"}))

	// check global env vars also
	assert.EqualValues(t, sortedArray([]string{"FOO", "BAR", "BAZ", "QUX"}), sortedArray(turboJSON.GlobalEnv))
	assert.EqualValues(t, sortedArray([]string{"somefile.txt"}), sortedArray(turboJSON.GlobalDeps))
}

// Helpers
func validateOutput(t *testing.T, turboJSON *TurboJSON, expectedPipeline map[string]TaskDefinition) {
	t.Helper()
	assertIsSorted(t, turboJSON.GlobalDeps, "Global Deps")
	assertIsSorted(t, turboJSON.GlobalEnv, "Global Env")
	validatePipeline(t, turboJSON.Pipeline, expectedPipeline)
}

func validatePipeline(t *testing.T, actual Pipeline, expected map[string]TaskDefinition) {
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
		actualTaskDefinition, ok := actual[taskName]
		if !ok {
			t.Errorf("missing expected task: %v", taskName)
		}
		assertIsSorted(t, actualTaskDefinition.Outputs.Inclusions, "Task output inclusions")
		assertIsSorted(t, actualTaskDefinition.Outputs.Exclusions, "Task output exclusions")
		assertIsSorted(t, actualTaskDefinition.EnvVarDependencies, "Task env vars")
		assertIsSorted(t, actualTaskDefinition.TopologicalDependencies, "Topo deps")
		assertIsSorted(t, actualTaskDefinition.TaskDependencies, "Task deps")
		assert.EqualValuesf(t, expectedTaskDefinition, actualTaskDefinition, "task definition mismatch for %v", taskName)
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
