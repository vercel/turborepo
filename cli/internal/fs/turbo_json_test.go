package fs

import (
	"os"
	"sort"
	"strings"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/vercel/turborepo/cli/internal/turbopath"
	"github.com/vercel/turborepo/cli/internal/util"
)

func Test_ReadTurboConfig(t *testing.T) {
	testDir := getTestDir(t, "correct")

	packageJSONPath := testDir.Join("package.json")
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
			Outputs:                 []string{"dist/**", ".next/**"},
			TopologicalDependencies: []string{"build"},
			EnvVarDependencies:      []string{},
			TaskDependencies:        []string{},
			ShouldCache:             true,
			OutputMode:              util.NewTaskOutput,
		},
		"lint": {
			Outputs:                 []string{},
			TopologicalDependencies: []string{},
			EnvVarDependencies:      []string{"MY_VAR"},
			TaskDependencies:        []string{},
			ShouldCache:             true,
			OutputMode:              util.NewTaskOutput,
		},
		"dev": {
			Outputs:                 defaultOutputs,
			EnvVarDependencies:      []string{},
			TopologicalDependencies: []string{},
			TaskDependencies:        []string{},
			ShouldCache:             false,
			OutputMode:              util.FullTaskOutput,
		},
		"publish": {
			Outputs:                 []string{"dist/**"},
			EnvVarDependencies:      []string{},
			TopologicalDependencies: []string{"publish"},
			TaskDependencies:        []string{"build", "admin#lint"},
			ShouldCache:             false,
			Inputs:                  []string{"build/**/*"},
			OutputMode:              util.FullTaskOutput,
		},
	}

	validateOutput(t, turboJSON.Pipeline, pipelineExpected)

	remoteCacheOptionsExpected := RemoteCacheOptions{"team_id", true}
	assert.EqualValues(t, remoteCacheOptionsExpected, turboJSON.RemoteCacheOptions)
}

func Test_ReadTurboConfig_Legacy(t *testing.T) {
	testDir := getTestDir(t, "legacy-only")

	packageJSONPath := testDir.Join("package.json")
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
			Outputs:                 []string{"dist/**/*", "build/**/*"},
			TopologicalDependencies: []string{},
			EnvVarDependencies:      []string{},
			TaskDependencies:        []string{},
			ShouldCache:             true,
			OutputMode:              util.FullTaskOutput,
		},
	}

	validateOutput(t, turboJSON.Pipeline, pipelineExpected)
	assert.Empty(t, turboJSON.RemoteCacheOptions)
}

func Test_ReadTurboConfig_BothCorrectAndLegacy(t *testing.T) {
	testDir := getTestDir(t, "both")

	packageJSONPath := testDir.Join("package.json")
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
			Outputs:                 []string{"dist/**", ".next/**"},
			TopologicalDependencies: []string{"build"},
			EnvVarDependencies:      []string{},
			TaskDependencies:        []string{},
			ShouldCache:             true,
			OutputMode:              util.NewTaskOutput,
		},
	}

	validateOutput(t, turboJSON.Pipeline, pipelineExpected)

	remoteCacheOptionsExpected := RemoteCacheOptions{"team_id", true}
	assert.EqualValues(t, remoteCacheOptionsExpected, turboJSON.RemoteCacheOptions)

	assert.Equal(t, rootPackageJSON.LegacyTurboConfig == nil, true)
}

func Test_ReadTurboConfig_InvalidEnvDeclarations1(t *testing.T) {
	testDir := getTestDir(t, "invalid-env-1")

	packageJSONPath := testDir.Join("package.json")
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

	packageJSONPath := testDir.Join("package.json")
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

	packageJSONPath := testDir.Join("package.json")
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

	packageJSONPath := testDir.Join("package.json")
	rootPackageJSON, pkgJSONReadErr := ReadPackageJSON(packageJSONPath)

	if pkgJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", pkgJSONReadErr)
	}

	turboJSON, turboJSONReadErr := ReadTurboConfig(testDir, rootPackageJSON)

	if turboJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", turboJSONReadErr)
	}

	pipeline := turboJSON.Pipeline
	assert.EqualValues(t, sortedArray(pipeline["task1"].EnvVarDependencies), sortedArray([]string{"A"}))
	assert.EqualValues(t, sortedArray(pipeline["task2"].EnvVarDependencies), sortedArray([]string{"A"}))
	assert.EqualValues(t, sortedArray(pipeline["task3"].EnvVarDependencies), sortedArray([]string{"A"}))
	assert.EqualValues(t, sortedArray(pipeline["task4"].EnvVarDependencies), sortedArray([]string{"A", "B"}))
	assert.EqualValues(t, sortedArray(pipeline["task6"].EnvVarDependencies), sortedArray([]string{"A", "B", "C", "D", "E", "F"}))
	assert.EqualValues(t, sortedArray(pipeline["task7"].EnvVarDependencies), sortedArray([]string{"A", "B", "C"}))
	assert.EqualValues(t, sortedArray(pipeline["task8"].EnvVarDependencies), sortedArray([]string{"A", "B", "C"}))
	assert.EqualValues(t, sortedArray(pipeline["task9"].EnvVarDependencies), sortedArray([]string{"A"}))
	assert.EqualValues(t, sortedArray(pipeline["task10"].EnvVarDependencies), sortedArray([]string{"A"}))
	assert.EqualValues(t, sortedArray(pipeline["task11"].EnvVarDependencies), sortedArray([]string{"A", "B"}))

	// check global env vars also
	assert.EqualValues(t, sortedArray([]string{"FOO", "BAR", "BAZ", "QUX"}), sortedArray(turboJSON.GlobalEnv))
	assert.EqualValues(t, sortedArray([]string{"somefile.txt"}), sortedArray(turboJSON.GlobalDeps))
}

// Helpers
func validateOutput(t *testing.T, actual Pipeline, expected map[string]TaskDefinition) {
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
		assert.EqualValuesf(t, expectedTaskDefinition, actualTaskDefinition, "task definition mismatch for %v", taskName)
	}

}

func getTestDir(t *testing.T, testName string) turbopath.AbsolutePath {
	defaultCwd, err := os.Getwd()
	if err != nil {
		t.Errorf("failed to get cwd: %v", err)
	}
	cwd, err := CheckedToAbsolutePath(defaultCwd)
	if err != nil {
		t.Fatalf("cwd is not an absolute directory %v: %v", defaultCwd, err)
	}

	return cwd.Join("testdata", testName)
}

func sortedArray(arr []string) []string {
	sort.Strings(arr)
	return arr
}
