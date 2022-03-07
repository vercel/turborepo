package config

import (
	"encoding/json"
	"io/ioutil"
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

// writeConfigFile writes config file at a path
func writeConfigFile(path string, config *TurborepoConfig) error {
	jsonBytes, marshallError := json.Marshal(config)
	if marshallError != nil {
		return marshallError
	}
	writeFilErr := ioutil.WriteFile(path, jsonBytes, 0644)
	if writeFilErr != nil {
		return writeFilErr
	}
	return nil
}

// WriteRepoConfigFile is used to write the portion of the config file that is saved
// within the repository itself.
func WriteRepoConfigFile(config *TurborepoConfig) error {
	fs.EnsureDir(filepath.Join(".turbo", "config.json"))
	path := filepath.Join(".turbo", "config.json")
	return writeConfigFile(path, config)
}

// WriteUserConfigFile writes a user config file. This may contain a token and so should
// not be saved within the repository to avoid committing sensitive data
func WriteUserConfigFile(config *TurborepoConfig) error {
	path, err := xdg.ConfigFile(filepath.Join("turborepo", "config.json"))
	if err != nil {
		return err
	}
	return writeConfigFile(path, config)
}

// ReadConfigFile reads a config file at a path
func ReadConfigFile(path string) (*TurborepoConfig, error) {
	var config = &TurborepoConfig{
		Token:    "",
		TeamId:   "",
		ApiUrl:   "https://vercel.com/api",
		LoginUrl: "https://vercel.com",
		TeamSlug: "",
	}
	b, err := ioutil.ReadFile(path)
	if err != nil {
		return config, err
	}
	jsonErr := json.Unmarshal(b, config)
	if jsonErr != nil {
		return config, jsonErr
	}
	if config.ApiUrl == "https://api.vercel.com" {
		config.ApiUrl = "https://vercel.com/api"
	}
	return config, nil
}

// ReadUserConfigFile reads a user config file
func ReadUserConfigFile() (*TurborepoConfig, error) {
	path, err := xdg.ConfigFile(filepath.Join("turborepo", "config.json"))
	if err != nil {
		return &TurborepoConfig{
			Token:    "",
			TeamId:   "",
			ApiUrl:   "https://vercel.com/api",
			LoginUrl: "https://vercel.com",
			TeamSlug: "",
		}, err
	}
	return ReadConfigFile(path)
}

// DeleteUserConfigFile deletes a user config file
func DeleteUserConfigFile() error {
	return WriteUserConfigFile(&TurborepoConfig{})
}
