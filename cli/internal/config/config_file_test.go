package config

import (
	"fmt"
	"testing"

	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbostate"
	"gotest.tools/v3/assert"
)

func TestReadRepoConfigWhenMissing(t *testing.T) {
	testDir := fs.AbsoluteSystemPathFromUpstream(t.TempDir()).UntypedJoin("config.json")
	args := &turbostate.ParsedArgsFromRust{
		CWD: "",
	}

	config, err := ReadRepoConfigFile(testDir, args)
	if err != nil {
		t.Errorf("got error reading non-existent config file: %v, want <nil>", err)
	}
	if config == nil {
		t.Error("got <nil>, wanted config value")
	}
}

func TestReadRepoConfigSetTeamAndAPIFlag(t *testing.T) {
	testConfigFile := fs.AbsoluteSystemPathFromUpstream(t.TempDir()).UntypedJoin("turborepo", "config.json")

	slug := "my-team-slug"
	apiURL := "http://my-login-url"
	args := &turbostate.ParsedArgsFromRust{
		CWD:  "",
		Team: slug,
		API:  apiURL,
	}

	teamID := "some-id"
	assert.NilError(t, testConfigFile.EnsureDir(), "EnsureDir")
	assert.NilError(t, testConfigFile.WriteFile([]byte(fmt.Sprintf(`{"teamId":"%v"}`, teamID)), 0644), "WriteFile")

	config, err := ReadRepoConfigFile(testConfigFile, args)
	if err != nil {
		t.Errorf("ReadRepoConfigFile err got %v, want <nil>", err)
	}
	remoteConfig := config.GetRemoteConfig("")
	if remoteConfig.TeamID != "" {
		t.Errorf("TeamID got %v, want <empty string>", remoteConfig.TeamID)
	}
	if remoteConfig.TeamSlug != slug {
		t.Errorf("TeamSlug got %v, want %v", remoteConfig.TeamSlug, slug)
	}
	if remoteConfig.APIURL != apiURL {
		t.Errorf("APIURL got %v, want %v", remoteConfig.APIURL, apiURL)
	}
}

func TestRepoConfigIncludesDefaults(t *testing.T) {
	testConfigFile := fs.AbsoluteSystemPathFromUpstream(t.TempDir()).UntypedJoin("turborepo", "config.json")
	args := &turbostate.ParsedArgsFromRust{
		CWD: "",
	}

	expectedTeam := "my-team"

	assert.NilError(t, testConfigFile.EnsureDir(), "EnsureDir")
	assert.NilError(t, testConfigFile.WriteFile([]byte(fmt.Sprintf(`{"teamSlug":"%v"}`, expectedTeam)), 0644), "WriteFile")

	config, err := ReadRepoConfigFile(testConfigFile, args)
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
	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())
	testConfigFile := repoRoot.UntypedJoin(".turbo", "config.json")
	args := &turbostate.ParsedArgsFromRust{
		CWD: "",
	}

	expectedTeam := "my-team"

	assert.NilError(t, testConfigFile.EnsureDir(), "EnsureDir")
	assert.NilError(t, testConfigFile.WriteFile([]byte(fmt.Sprintf(`{"teamSlug":"%v"}`, expectedTeam)), 0644), "WriteFile")

	initial, err := ReadRepoConfigFile(testConfigFile, args)
	assert.NilError(t, err, "GetRepoConfig")
	// setting the teamID should clear the slug, since it may have been from an old team
	expectedTeamID := "my-team-id"
	err = initial.SetTeamID(expectedTeamID)
	assert.NilError(t, err, "SetTeamID")

	config, err := ReadRepoConfigFile(testConfigFile, args)
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
	configPath := fs.AbsoluteSystemPathFromUpstream(t.TempDir()).UntypedJoin("turborepo", "config.json")
	args := &turbostate.ParsedArgsFromRust{
		CWD: "",
	}

	// Non-existent config file should get empty values
	userConfig, err := ReadUserConfigFile(configPath, args)
	assert.NilError(t, err, "readUserConfigFile")
	assert.Equal(t, userConfig.Token(), "")
	assert.Equal(t, userConfig.path, configPath)

	expectedToken := "my-token"
	err = userConfig.SetToken(expectedToken)
	assert.NilError(t, err, "SetToken")

	config, err := ReadUserConfigFile(configPath, args)
	assert.NilError(t, err, "readUserConfigFile")
	assert.Equal(t, config.Token(), expectedToken)

	err = config.Delete()
	assert.NilError(t, err, "deleteConfigFile")
	assert.Equal(t, configPath.FileExists(), false, "config file should be deleted")

	final, err := ReadUserConfigFile(configPath, args)
	assert.NilError(t, err, "readUserConfigFile")
	assert.Equal(t, final.Token(), "")
	assert.Equal(t, configPath.FileExists(), false, "config file should be deleted")
}

func TestUserConfigFlags(t *testing.T) {
	configPath := fs.AbsoluteSystemPathFromUpstream(t.TempDir()).UntypedJoin("turborepo", "config.json")
	args := &turbostate.ParsedArgsFromRust{
		CWD:   "",
		Token: "my-token",
	}

	userConfig, err := ReadUserConfigFile(configPath, args)
	assert.NilError(t, err, "readUserConfigFile")
	assert.Equal(t, userConfig.Token(), "my-token")
	assert.Equal(t, userConfig.path, configPath)
}
