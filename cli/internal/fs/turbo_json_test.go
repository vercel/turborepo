package fs

import (
	"os"
	"strings"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/vercel/turborepo/cli/internal/util"
)

func Test_ReadTurboConfig(t *testing.T) {
	defaultCwd, err := os.Getwd()
	if err != nil {
		t.Errorf("failed to get cwd: %v", err)
	}
	cwd, err := CheckedToAbsolutePath(defaultCwd)
	if err != nil {
		t.Fatalf("cwd is not an absolute directory %v: %v", defaultCwd, err)
	}

	rootDir := "testdata"
	turboJSONPath := cwd.Join(rootDir)
	packageJSONPath := cwd.Join(rootDir, "package.json")
	rootPackageJSON, pkgJSONReadErr := ReadPackageJSON(packageJSONPath.ToStringDuringMigration())

	if pkgJSONReadErr != nil {
		t.Fatalf("invalid parse: %#v", pkgJSONReadErr)
	}

	turboJSON, turboJSONReadErr := ReadTurboConfig(turboJSONPath, rootPackageJSON)

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

	remoteCacheOptionsExpected := RemoteCacheOptions{"team_id", true}
	if len(turboJSON.Pipeline) != len(pipelineExpected) {
		expectedKeys := []string{}
		for k := range pipelineExpected {
			expectedKeys = append(expectedKeys, k)
		}
		actualKeys := []string{}
		for k := range turboJSON.Pipeline {
			actualKeys = append(actualKeys, k)
		}
		t.Errorf("pipeline tasks mismatch. got %v, want %v", strings.Join(actualKeys, ","), strings.Join(expectedKeys, ","))
	}
	for taskName, expectedTaskDefinition := range pipelineExpected {
		actualTaskDefinition, ok := turboJSON.Pipeline[taskName]
		if !ok {
			t.Errorf("missing expected task: %v", taskName)
		}
		assert.EqualValuesf(t, expectedTaskDefinition, actualTaskDefinition, "task definition mismatch for %v", taskName)
	}
	assert.EqualValues(t, remoteCacheOptionsExpected, turboJSON.RemoteCacheOptions)
}
