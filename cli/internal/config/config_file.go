package config

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"

	"github.com/adrg/xdg"
	"github.com/spf13/afero"
	"github.com/vercel/turborepo/cli/internal/fs"
)

// TurborepoConfig is a configuration object for the logged-in turborepo.com user
type TurborepoConfig struct {
	// Token is a bearer token
	Token string `json:"token,omitempty"`
	// Team id
	TeamId string `json:"teamId,omitempty"`
	// ApiUrl is the backend url (defaults to api.vercel.com)
	ApiUrl string `json:"apiUrl,omitempty" envconfig:"api"`
	// LoginUrl is the login url (defaults to vercel.com)
	LoginUrl string `json:"loginUrl,omitempty" envconfig:"login"`
	// Owner slug
	TeamSlug string `json:"teamSlug,omitempty" envconfig:"team"`
}

func defaultUserConfig() *TurborepoConfig {
	return &TurborepoConfig{
		ApiUrl:   "https://vercel.com/api",
		LoginUrl: "https://vercel.com",
	}
}

func defaultRepoConfig() *TurborepoConfig {
	return &TurborepoConfig{
		ApiUrl:   "https://vercel.com/api",
		LoginUrl: "https://vercel.com",
	}
}

// writeConfigFile writes config file at a path
func writeConfigFile(fsys afero.Fs, path fs.AbsolutePath, config *TurborepoConfig) error {
	jsonBytes, marshallError := json.Marshal(config)
	if marshallError != nil {
		return marshallError
	}
	writeFilErr := fs.WriteFile(fsys, path, jsonBytes, 0644)
	if writeFilErr != nil {
		return writeFilErr
	}
	return nil
}

// WriteRepoConfigFile is used to write the portion of the config file that is saved
// within the repository itself.
func WriteRepoConfigFile(fsys afero.Fs, repoRoot fs.AbsolutePath, toWrite *TurborepoConfig) error {
	path := repoRoot.Join(".turbo", "config.json")
	err := fs.EnsureDirFS(fsys, path)
	if err != nil {
		return err
	}
	return writeConfigFile(fsys, path, toWrite)
}

func userConfigPath(fsys afero.Fs) (fs.AbsolutePath, error) {
	path, err := xdg.ConfigFile(filepath.Join("turborepo", "config.json"))
	if err != nil {
		return "", err
	}
	absPath, err := fs.CheckedToAbsolutePath(path)
	if err != nil {
		return "", err
	}
	return absPath, nil
}

// WriteUserConfigFile writes the given configuration to a user-specific
// configuration file. This is for values that are not shared with a team, such
// as credentials.
func WriteUserConfigFile(fsys afero.Fs, config *TurborepoConfig) error {
	path, err := userConfigPath(fsys)
	if err != nil {
		return err
	}
	return writeConfigFile(fsys, path, config)
}

// readConfigFile reads a config file at a path
func readConfigFile(fsys afero.Fs, path fs.AbsolutePath, defaults func() *TurborepoConfig) (*TurborepoConfig, error) {
	b, err := fs.ReadFile(fsys, path)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return nil, nil
		}
		return nil, err
	}
	config := defaults()
	jsonErr := json.Unmarshal(b, config)
	if jsonErr != nil {
		return nil, jsonErr
	}
	if config.ApiUrl == "https://api.vercel.com" {
		config.ApiUrl = "https://vercel.com/api"
	}
	return config, nil
}

// ReadUserConfigFile reads a user config file
func ReadUserConfigFile(fsys afero.Fs) (*TurborepoConfig, error) {
	path, err := userConfigPath(fsys)
	if err != nil {
		return nil, err
	}
	return readConfigFile(fsys, path, defaultUserConfig)
}

// ReadRepoConfigFile reads the user-specific configuration values
func ReadRepoConfigFile(fsys afero.Fs, repoRoot fs.AbsolutePath) (*TurborepoConfig, error) {
	path := repoRoot.Join(".turbo", "config.json")
	return readConfigFile(fsys, path, defaultRepoConfig)
}

// DeleteUserConfigFile deletes a user config file
func DeleteUserConfigFile(fsys afero.Fs) error {
	path, err := userConfigPath(fsys)
	if err != nil {
		return err
	}
	return fs.RemoveFile(fsys, path)
}
