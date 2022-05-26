package config

import (
	"encoding/json"
	"os"
	"testing"

	"github.com/spf13/afero"
	"github.com/vercel/turborepo/cli/internal/fs"
)

func TestReadRepoConfigWhenMissing(t *testing.T) {
	fsys := afero.NewMemMapFs()
	cwd, err := fs.GetCwd()
	if err != nil {
		t.Fatalf("getting cwd: %v", err)
	}

	config, err := ReadRepoConfigFile(fsys, cwd)
	if err != nil {
		t.Errorf("got error reading non-existent config file: %v, want <nil>", err)
	}
	if config != nil {
		t.Errorf("got config value %v, wanted <nil>", config)
	}
}

func writePartialInitialConfig(t *testing.T, fsys afero.Fs, repoRoot fs.AbsolutePath) *TurborepoConfig {
	path := repoRoot.Join(".turbo", "config.json")
	initial := &TurborepoConfig{
		TeamSlug: "my-team",
	}
	toWrite, err := json.Marshal(initial)
	if err != nil {
		t.Fatalf("marshal config: %v", err)
	}
	err = fs.WriteFile(fsys, path, toWrite, os.ModePerm)
	if err != nil {
		t.Fatalf("writing config file: %v", err)
	}
	return initial
}

func TestRepoConfigIncludesDefaults(t *testing.T) {
	fsys := afero.NewMemMapFs()
	cwd, err := fs.GetCwd()
	if err != nil {
		t.Fatalf("getting cwd: %v", err)
	}

	initial := writePartialInitialConfig(t, fsys, cwd)

	config, err := ReadRepoConfigFile(fsys, cwd)
	if err != nil {
		t.Errorf("ReadRepoConfigFile err got %v, want <nil>", err)
	}
	defaultConfig := defaultRepoConfig()
	if config.ApiUrl != defaultConfig.ApiUrl {
		t.Errorf("api url got %v, want %v", config.ApiUrl, defaultConfig.ApiUrl)
	}
	if config.TeamSlug != initial.TeamSlug {
		t.Errorf("team slug got %v, want %v", config.TeamSlug, initial.TeamSlug)
	}
}

func TestWriteRepoConfig(t *testing.T) {
	fsys := afero.NewMemMapFs()
	cwd, err := fs.GetCwd()
	if err != nil {
		t.Fatalf("getting cwd: %v", err)
	}

	initial := &TurborepoConfig{}
	initial.TeamSlug = "my-team"
	err = WriteRepoConfigFile(fsys, cwd, initial)
	if err != nil {
		t.Errorf("WriteRepoConfigFile got %v, want <nil>", err)
	}

	config, err := ReadRepoConfigFile(fsys, cwd)
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

func TestReadUserConfigWhenMissing(t *testing.T) {
	fsys := afero.NewMemMapFs()
	config, err := ReadUserConfigFile(fsys)
	if err != nil {
		t.Errorf("ReadUserConfig err got %v, want <nil>", err)
	}
	if config != nil {
		t.Errorf("ReadUserConfig on non-existent file got %v, want <nil>", config)
	}
}

func TestWriteUserConfig(t *testing.T) {
	fsys := afero.NewMemMapFs()
	initial := defaultUserConfig()
	initial.Token = "my-token"
	initial.ApiUrl = "https://api.vercel.com" // should be overridden
	err := WriteUserConfigFile(fsys, initial)
	if err != nil {
		t.Errorf("WriteUserConfigFile err got %v, want <nil>", err)
	}

	config, err := ReadUserConfigFile(fsys)
	if err != nil {
		t.Errorf("ReadUserConfig err got %v, want <nil>", err)
	}
	if config.Token != initial.Token {
		t.Errorf("Token got %v want %v", config.Token, initial.Token)
	}
	// Verify that our legacy ApiUrl was upgraded
	defaultConfig := defaultUserConfig()
	if config.ApiUrl != defaultConfig.ApiUrl {
		t.Errorf("ApiUrl got %v, want %v", config.ApiUrl, defaultConfig.ApiUrl)
	}

	err = DeleteUserConfigFile(fsys)
	if err != nil {
		t.Errorf("DeleteUserConfigFile err got %v, want <nil>", err)
	}

	missing, err := ReadUserConfigFile(fsys)
	if err != nil {
		t.Errorf("ReadUserConfig err got %v, want <nil>", err)
	}
	if missing != nil {
		t.Errorf("reading deleted config got %v, want <nil>", missing)
	}
}
