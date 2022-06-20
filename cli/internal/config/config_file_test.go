package config

import (
	"fmt"
	"os"
	"testing"

	"github.com/vercel/turborepo/cli/internal/fs"
)

func backupExistingConfig(t *testing.T) fs.AbsolutePath {
	t.Helper()
	path, err := createUserConfigPath()
	if err != nil {
		t.Fatalf("failed to get user config path: %v", err)
	}
	configDir := path.Dir()
	if configDir.DirExists() {
		backup := fs.AbsolutePathFromUpstream(t.TempDir()).Join("config_test")
		if err := configDir.Rename(backup); err != nil {
			t.Fatalf("failed to backup %v to %v: %v", configDir, backup, err)
		}
		t.Cleanup(func() {
			// don't have a live testing instance here, use panic instead
			if err := configDir.Remove(); err != nil {
				panic(fmt.Sprintf("failed to remove test config dir %v: %v", configDir, err))
			}
			if err := backup.Rename(configDir); err != nil {
				panic(fmt.Sprintf("failed to restore %v from %v", configDir, backup))
			}
		})
	}
	return path
}

func Test_UserConfigPath(t *testing.T) {
	// XDG is not filesystem aware. Clean up first.
	path := backupExistingConfig(t)

	getConfigPath, getConfigPathErr := getUserConfigPath()
	if getConfigPathErr != nil {
		t.Errorf("failed to run getUserConfigPath: %v", getConfigPathErr)
	}
	// We just cleaned up the existing config, if it existed.
	// We should not currenctly have one
	if getConfigPath != "" {
		t.Fatalf("expected to not find a config file, got %v", getConfigPath)
	}

	// The main thing we want to do is make sure that we don't have side effects.
	// We know where it would attempt to create a directory already.
	getConfigPath = path

	getConfigDir := getConfigPath.Dir()
	getCheck, _ := os.Stat(getConfigDir.ToString())
	if getCheck != nil {
		t.Error("getUserConfigPath() had side effects.")
	}

	createConfigPath, createErr := createUserConfigPath()
	if createErr != nil {
		t.Errorf("createUserConfigPath() errored: %v.", createErr)
	}
	createConfigDir := createConfigPath.Dir()
	createCheck, _ := os.Stat(createConfigDir.ToString())
	if createCheck == nil {
		t.Error("createUserConfigPath() did not create the path.")
	}
}

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

func TestReadUserConfigWhenMissing(t *testing.T) {
	// Make sure it actually doesn't exist first.
	path, _ := getUserConfigPath()
	if path.FileExists() {
		// remove the file.
		err := path.Remove()
		if err != nil {
			t.Error("User config path unable to be removed.")
		}
	}

	// Proceed with the test.
	config, err := ReadUserConfigFile()
	if err != nil {
		t.Errorf("ReadUserConfig err got %v, want <nil>", err)
	}
	if config != nil {
		t.Errorf("ReadUserConfig on non-existent file got %v, want <nil>", config)
	}
}

func TestWriteUserConfig(t *testing.T) {
	initial := defaultUserConfig()
	initial.Token = "my-token"
	initial.ApiUrl = "https://api.vercel.com" // should be overridden

	err := WriteUserConfigFile(initial)
	if err != nil {
		t.Errorf("WriteUserConfigFile err got %v, want <nil>", err)
	}

	config, err := ReadUserConfigFile()
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

	err = DeleteUserConfigFile()
	if err != nil {
		t.Errorf("DeleteUserConfigFile err got %v, want <nil>", err)
	}

	missing, err := ReadUserConfigFile()
	if err != nil {
		t.Errorf("ReadUserConfig err got %v, want <nil>", err)
	}
	if missing != nil {
		t.Errorf("reading deleted config got %v, want <nil>", missing)
	}
}
