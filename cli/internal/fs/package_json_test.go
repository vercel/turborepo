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
	boolRef := false
	pipelineExpected := map[string]Pipeline{"dev": {nil, &boolRef, nil, PPipeline{nil, &boolRef, nil}}}
	remoteCacheOptionsExpected := RemoteCacheOptions{"team_id", SignatureOptions{true, "key"}}
	assert.EqualValues(t, pipelineExpected, turboConfig.Pipeline)
	assert.EqualValues(t, remoteCacheOptionsExpected, turboConfig.RemoteCacheOptions)
}
