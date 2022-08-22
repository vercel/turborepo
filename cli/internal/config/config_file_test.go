package config

import (
	"testing"

	"github.com/vercel/turborepo/cli/internal/fs"
	"gotest.tools/v3/assert"
)

func TestReadRepoConfigWhenMissing(t *testing.T) {
	testDir := fs.AbsolutePath(t.TempDir())

	config, err := ReadRepoConfigFile(testDir)
	if err != nil {
		t.Errorf("got error reading non-existent config file: %v, want <nil>", err)
	}
	if config != nil {
		t.Errorf("got config value %v, wanted <nil>", config)
	}
}

func TestRepoConfigIncludesDefaults(t *testing.T) {
	testDir := fs.AbsolutePath(t.TempDir())

	customConfig := &TurborepoConfig{
		TeamSlug: "my-team",
	}

	initialWriteErr := WriteRepoConfigFile(testDir, customConfig)
	if initialWriteErr != nil {
		t.Errorf("Failed to set up test: %v", initialWriteErr)
	}

	config, err := ReadRepoConfigFile(testDir)
	if err != nil {
		t.Errorf("ReadRepoConfigFile err got %v, want <nil>", err)
	}

	defaultConfig := defaultRepoConfig()
	if config.ApiUrl != defaultConfig.ApiUrl {
		t.Errorf("api url got %v, want %v", config.ApiUrl, defaultConfig.ApiUrl)
	}
	if config.TeamSlug != customConfig.TeamSlug {
		t.Errorf("team slug got %v, want %v", config.TeamSlug, customConfig.TeamSlug)
	}
}

func TestWriteRepoConfig(t *testing.T) {
	testDir := fs.AbsolutePath(t.TempDir())

	initial := &TurborepoConfig{}
	initial.TeamSlug = "my-team"
	err := WriteRepoConfigFile(testDir, initial)
	if err != nil {
		t.Errorf("WriteRepoConfigFile got %v, want <nil>", err)
	}

	config, err := ReadRepoConfigFile(testDir)
	if err != nil {
		t.Errorf("ReadRepoConfig err got %v, want <nil>", err)
	}

	if config.TeamSlug != initial.TeamSlug {
		t.Errorf("TeamSlug got %v want %v", config.TeamSlug, initial.TeamSlug)
	}
	defaultConfig := defaultRepoConfig()
	if config.ApiUrl != defaultConfig.ApiUrl {
		t.Errorf("ApiUrl got %v, want %v", config.ApiUrl, defaultConfig.ApiUrl)
	}
}

func TestWriteUserConfig(t *testing.T) {
	configPath := fs.AbsolutePathFromUpstream(t.TempDir()).Join("turborepo", "config.json")
	// Non-existent config file should get empty values
	userConfig, err := ReadUserConfigFile(configPath)
	assert.NilError(t, err, "readUserConfigFile")
	assert.Equal(t, userConfig.Token(), "")
	assert.Equal(t, userConfig.path, configPath)

	expectedToken := "my-token"
	err = userConfig.SetToken(expectedToken)
	assert.NilError(t, err, "SetToken")

	config, err := ReadUserConfigFile(configPath)
	assert.NilError(t, err, "readUserConfigFile")
	assert.Equal(t, config.Token(), expectedToken)

	err = config.Delete()
	assert.NilError(t, err, "deleteConfigFile")
	assert.Equal(t, configPath.FileExists(), false, "config file should be deleted")

	final, err := ReadUserConfigFile(configPath)
	assert.NilError(t, err, "readUserConfigFile")
	assert.Equal(t, final.Token(), "")
	assert.Equal(t, configPath.FileExists(), false, "config file should be deleted")
}
