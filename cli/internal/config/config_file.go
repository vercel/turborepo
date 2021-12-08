package config

import (
	"encoding/json"
	"io/ioutil"
	"path/filepath"

	"github.com/adrg/xdg"
)

// TurborepoConfig is a configuration object for the logged-in turborepo.com user
type TurborepoConfig struct {
	// Token is a bearer token
	Token string `json:"token,omitempty"`
	// Team id
	TeamId string `json:"teamId,omitempty"`
	// ApiUrl is the backend url (defaults to turborepo.com)
	ApiUrl string `json:"apiUrl,omitempty" envconfig:"api"`
	// Owner slug
	TeamSlug string `json:"teamSlug,omitempty" envconfig:"team"`
}

// WriteUserConfigFile writes config file at a oath
func WriteConfigFile(path string, config *TurborepoConfig) error {
	jsonBytes, marhsallError := json.Marshal(config)
	if marhsallError != nil {
		return marhsallError
	}
	writeFilErr := ioutil.WriteFile(path, jsonBytes, 0644)
	if writeFilErr != nil {
		return writeFilErr
	}
	return nil
}

// WriteUserConfigFile writes a user config file
func WriteUserConfigFile(config *TurborepoConfig) error {
	path, err := xdg.ConfigFile(filepath.Join("turborepo", "config.json"))
	if err != nil {
		return err
	}
	return WriteConfigFile(path, config)
}

// ReadConfigFile reads a config file at a path
func ReadConfigFile(path string) (*TurborepoConfig, error) {
	var config = &TurborepoConfig{
		Token:    "",
		TeamId:   "",
		ApiUrl:   "https://api.vercel.com",
		TeamSlug: "",
	}
	b, err := ioutil.ReadFile(path)
	if err != nil {
		return config, err
	}
	jsonErr := json.Unmarshal(b, &config)
	if jsonErr != nil {
		return config, jsonErr
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
			ApiUrl:   "https://api.vercel.com",
			TeamSlug: "",
		}, err
	}
	return ReadConfigFile(path)
}

// DeleteUserConfigFile deletes a user  config file
func DeleteUserConfigFile() error {
	return WriteUserConfigFile(&TurborepoConfig{})
}
