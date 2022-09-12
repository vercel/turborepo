package config

import (
	"fmt"
	"testing"

	"github.com/spf13/pflag"
	"github.com/vercel/turborepo/cli/internal/fs"
	"gotest.tools/v3/assert"
)

func TestReadRepoConfigWhenMissing(t *testing.T) {
	testDir := fs.AbsolutePathFromUpstream(t.TempDir()).Join("config.json")
	flags := pflag.NewFlagSet("test-flags", pflag.ContinueOnError)
	AddRepoConfigFlags(flags)

	config, err := ReadRepoConfigFile(testDir, flags)
	if err != nil {
		t.Errorf("got error reading non-existent config file: %v, want <nil>", err)
	}
	if config == nil {
		t.Error("got <nil>, wanted config value")
	}
}

func TestRepoConfigIncludesDefaults(t *testing.T) {
	testConfigFile := fs.AbsolutePathFromUpstream(t.TempDir()).Join("turborepo", "config.json")
	flags := pflag.NewFlagSet("test-flags", pflag.ContinueOnError)
	AddRepoConfigFlags(flags)

	expectedTeam := "my-team"

	assert.NilError(t, testConfigFile.EnsureDir(), "EnsureDir")
	assert.NilError(t, testConfigFile.WriteFile([]byte(fmt.Sprintf(`{"teamSlug":"%v"}`, expectedTeam)), 0644), "WriteFile")

	config, err := ReadRepoConfigFile(testConfigFile, flags)
	if err != nil {
		t.Errorf("ReadRepoConfigFile err got %v, want <nil>", err)
	}

	remoteConfig := config.GetRemoteConfig("")
	if remoteConfig.APIURL != _defaultAPIURL {
		t.Errorf("api url got %v, want %v", remoteConfig.APIURL, _defaultAPIURL)
	}
	if remoteConfig.TeamSlug != expectedTeam {
		t.Errorf("team slug got %v, want %v", remoteConfig.TeamSlug, expectedTeam)
	}
}

func TestWriteRepoConfig(t *testing.T) {
	repoRoot := fs.AbsolutePathFromUpstream(t.TempDir())
	testConfigFile := repoRoot.Join(".turbo", "config.json")
	flags := pflag.NewFlagSet("test-flags", pflag.ContinueOnError)
	AddRepoConfigFlags(flags)

	expectedTeam := "my-team"

	assert.NilError(t, testConfigFile.EnsureDir(), "EnsureDir")
	assert.NilError(t, testConfigFile.WriteFile([]byte(fmt.Sprintf(`{"teamSlug":"%v"}`, expectedTeam)), 0644), "WriteFile")

	initial, err := ReadRepoConfigFile(testConfigFile, flags)
	assert.NilError(t, err, "GetRepoConfig")
	// setting the teamID should clear the slug, since it may have been from an old team
	expectedTeamID := "my-team-id"
	err = initial.SetTeamID(expectedTeamID)
	assert.NilError(t, err, "SetTeamID")

	config, err := ReadRepoConfigFile(testConfigFile, flags)
	if err != nil {
		t.Errorf("ReadRepoConfig err got %v, want <nil>", err)
	}

	remoteConfig := config.GetRemoteConfig("")
	if remoteConfig.TeamSlug != "" {
		t.Errorf("Expected TeamSlug to be cleared, got %v", remoteConfig.TeamSlug)
	}
	if remoteConfig.TeamID != expectedTeamID {
		t.Errorf("TeamID got %v, want %v", remoteConfig.TeamID, expectedTeamID)
	}
}

func TestWriteUserConfig(t *testing.T) {
	configPath := fs.AbsolutePathFromUpstream(t.TempDir()).Join("turborepo", "config.json")
	flags := pflag.NewFlagSet("test-flags", pflag.ContinueOnError)
	AddUserConfigFlags(flags)
	// Non-existent config file should get empty values
	userConfig, err := ReadUserConfigFile(configPath, flags)
	assert.NilError(t, err, "readUserConfigFile")
	assert.Equal(t, userConfig.Token(), "")
	assert.Equal(t, userConfig.path, configPath)

	expectedToken := "my-token"
	err = userConfig.SetToken(expectedToken)
	assert.NilError(t, err, "SetToken")

	config, err := ReadUserConfigFile(configPath, flags)
	assert.NilError(t, err, "readUserConfigFile")
	assert.Equal(t, config.Token(), expectedToken)

	err = config.Delete()
	assert.NilError(t, err, "deleteConfigFile")
	assert.Equal(t, configPath.FileExists(), false, "config file should be deleted")

	final, err := ReadUserConfigFile(configPath, flags)
	assert.NilError(t, err, "readUserConfigFile")
	assert.Equal(t, final.Token(), "")
	assert.Equal(t, configPath.FileExists(), false, "config file should be deleted")
}

func TestUserConfigFlags(t *testing.T) {
	configPath := fs.AbsolutePathFromUpstream(t.TempDir()).Join("turborepo", "config.json")
	flags := pflag.NewFlagSet("test-flags", pflag.ContinueOnError)
	AddUserConfigFlags(flags)

	assert.NilError(t, flags.Set("token", "my-token"), "set flag")
	userConfig, err := ReadUserConfigFile(configPath, flags)
	assert.NilError(t, err, "readUserConfigFile")
	assert.Equal(t, userConfig.Token(), "my-token")
	assert.Equal(t, userConfig.path, configPath)
}
