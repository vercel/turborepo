package fs

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/stretchr/testify/assert"
)

func Test_ParseTurboConfigJson(t *testing.T) {
	defaultCwd, err := os.Getwd()
	if err != nil {
		t.Errorf("failed to get cwd: %v", err)
	}
	turboJSONPath := filepath.Join(defaultCwd, "testdata", "turbo.json")
	turboConfig, err := ReadTurboConfigJSON(turboJSONPath)
	if err != nil {
		t.Fatalf("invalid parse: %#v", err)
	}

	pipelineExpected := map[string]Pipeline{
		"build": {
			Outputs:                 []string{"dist/**", ".next/**"},
			TopologicalDependencies: []string{"build"},
			EnvVarDependencies:      []string{},
			TaskDependencies:        []string{},
			ShouldCache:             true,
		},
		"lint": {
			Outputs:                 []string{},
			TopologicalDependencies: []string{},
			EnvVarDependencies:      []string{"MY_VAR"},
			TaskDependencies:        []string{},
			ShouldCache:             true,
		},
		"dev": {
			Outputs:                 defaultOutputs,
			EnvVarDependencies:      []string{},
			TopologicalDependencies: []string{},
			TaskDependencies:        []string{},
			ShouldCache:             false,
		},
		"publish": {
			Outputs:                 []string{"dist/**"},
			EnvVarDependencies:      []string{},
			TopologicalDependencies: []string{"publish"},
			TaskDependencies:        []string{"build", "admin#lint"},
			ShouldCache:             false,
			Inputs:                  []string{"build/**/*"},
		},
	}

	remoteCacheOptionsExpected := RemoteCacheOptions{"team_id", true}
	if len(turboConfig.Pipeline) != len(pipelineExpected) {
		expectedKeys := []string{}
		for k := range pipelineExpected {
			expectedKeys = append(expectedKeys, k)
		}
		actualKeys := []string{}
		for k := range turboConfig.Pipeline {
			actualKeys = append(actualKeys, k)
		}
		t.Errorf("pipeline tasks mismatch. got %v, want %v", strings.Join(actualKeys, ","), strings.Join(expectedKeys, ","))
	}
	for taskName, expectedTaskDefinition := range pipelineExpected {
		actualTaskDefinition, ok := turboConfig.Pipeline[taskName]
		if !ok {
			t.Errorf("missing expected task: %v", taskName)
		}
		assert.EqualValuesf(t, expectedTaskDefinition, actualTaskDefinition, "task definition mismatch for %v", taskName)
	}
	assert.EqualValues(t, remoteCacheOptionsExpected, turboConfig.RemoteCacheOptions)
}
