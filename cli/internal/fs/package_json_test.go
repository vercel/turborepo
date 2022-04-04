package fs

import (
	"os"
	"path/filepath"
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
	BoolFalse := false

	build := Pipeline{
		Outputs:   []string{"dist/**", ".next/**"},
		DependsOn: []string{"^build"},
		PPipeline: PPipeline{
			Outputs:   &[]string{"dist/**", ".next/**"},
			DependsOn: []string{"^build"},
		},
	}
	lint := Pipeline{
		Outputs:   []string{},
		PPipeline: PPipeline{Outputs: &[]string{}},
	}
	dev := Pipeline{
		Cache: &BoolFalse,
		PPipeline: PPipeline{
			Cache: &BoolFalse,
		},
	}
	pipelineExpected := map[string]Pipeline{"build": build, "lint": lint, "dev": dev}

	remoteCacheOptionsExpected := RemoteCacheOptions{"team_id", true}
	assert.EqualValues(t, pipelineExpected, turboConfig.Pipeline)
	assert.EqualValues(t, remoteCacheOptionsExpected, turboConfig.RemoteCacheOptions)
}
