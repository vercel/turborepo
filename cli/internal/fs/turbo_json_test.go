package fs

import (
	"os"
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
