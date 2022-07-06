package config

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"

	"github.com/adrg/xdg"
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
func writeConfigFile(path fs.AbsolutePath, config *TurborepoConfig) error {
	jsonBytes, marshallError := json.Marshal(config)
	if marshallError != nil {
		return marshallError
	}
	writeFilErr := path.WriteFile(jsonBytes, 0644)
	if writeFilErr != nil {
		return writeFilErr
	}
	return nil
}

// WriteRepoConfigFile is used to write the portion of the config file that is saved
// within the repository itself.
func WriteRepoConfigFile(repoRoot fs.AbsolutePath, toWrite *TurborepoConfig) error {
	path := repoRoot.Join(".turbo", "config.json")
	err := path.EnsureDir()
	if err != nil {
		return err
	}
	return writeConfigFile(path, toWrite)
}

func getUserConfigPath() (fs.AbsolutePath, error) {
	path, err := xdg.SearchConfigFile(filepath.Join("turborepo", "config.json"))
	// Not finding an existing config file is not an error.
	// We simply bail with no path, and no error.
	if err != nil {
		return "", nil
	}
	absPath, err := fs.CheckedToAbsolutePath(path)
	if err != nil {
		return "", err
	}
	return absPath, nil
}

func createUserConfigPath() (fs.AbsolutePath, error) {
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
func WriteUserConfigFile(config *TurborepoConfig) error {
	path, err := createUserConfigPath()
	if err != nil {
		return err
	}
	return writeConfigFile(path, config)
}

// readConfigFile reads a config file at a path
func readConfigFile(path fs.AbsolutePath, defaults func() *TurborepoConfig) (*TurborepoConfig, error) {
	b, err := path.ReadFile()
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
func ReadUserConfigFile() (*TurborepoConfig, error) {
	path, err := getUserConfigPath()

	// Check the error first, that means we got a hit, but failed on path conversion.
	if err != nil {
		return nil, err
	}

	// Otherwise, we just didn't find anything, which isn't an error.
	if path == "" {
		return nil, nil
	}

	// Found something!
	return readConfigFile(path, defaultUserConfig)
}

// ReadRepoConfigFile reads the user-specific configuration values
func ReadRepoConfigFile(repoRoot fs.AbsolutePath) (*TurborepoConfig, error) {
	path := repoRoot.Join(".turbo", "config.json")
	return readConfigFile(path, defaultRepoConfig)
}

// DeleteUserConfigFile deletes a user config file
func DeleteUserConfigFile() error {
	path, err := getUserConfigPath()

	// Check the error first, that means we got a hit, but failed on path conversion.
	if err != nil {
		return err
	}

	// Otherwise, we just didn't find anything, which isn't an error.
	if path == "" {
		return nil
	}

	// Found a config file!
	return path.Remove()
}
