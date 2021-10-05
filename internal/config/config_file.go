package config

import (
	"encoding/json"
	"io/ioutil"

	"github.com/adrg/xdg"
)

// ConfigFilePath is the path to the xdg configuration file
const ConfigFilePath = "turborepo/config.json"

// TurborepoConfig is a configuration object for the logged-in turborepo.com user
type TurborepoConfig struct {
	// Token is a bearer token
	Token string `json:"token,omitempty"`
	// ProjectId is the turborepo.com project id
	ProjectId string `json:"projectId,omitempty"`
	// Team is the turborepo.com team id
	TeamId string `json:"teamId,omitempty"`
	// ApiUrl is the backend url (defaults to turborepo.com)
	ApiUrl string `json:"apiUrl,omitempty"`
	// Turborepo.com team slug
	TeamSlug string `json:"teamSlug,omitempty" envconfig:"team"`
	// Turborepo.com project slug
	ProjectSlug string `json:"projectSlug,omitempty" envconfig:"project"`
}

// WriteUserConfigFile writes config file at a oath
func WriteConfigFile(path string, config *TurborepoConfig) error {
	yamlBytes, marhsallError := json.Marshal(config)
	if marhsallError != nil {
		return marhsallError
	}
	writeFilErr := ioutil.WriteFile(path, yamlBytes, 0644)
	if writeFilErr != nil {
		return writeFilErr
	}
	return nil
}

// WriteUserConfigFile writes a user config file
func WriteUserConfigFile(config *TurborepoConfig) error {
	path, err := xdg.ConfigFile(ConfigFilePath)
	if err != nil {
		return err
	}
	return WriteConfigFile(path, config)
}

// ReadConfigFile reads a config file at a path
func ReadConfigFile(path string) (*TurborepoConfig, error) {
	var config = &TurborepoConfig{
		Token:       "",
		ProjectId:   "",
		TeamId:      "",
		ApiUrl:      "https://beta.turborepo.com/api",
		TeamSlug:    "",
		ProjectSlug: "",
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
	path, err := xdg.ConfigFile(ConfigFilePath)
	if err != nil {
		return &TurborepoConfig{
			Token:       "",
			ProjectId:   "",
			TeamId:      "",
			ApiUrl:      "https://beta.turborepo.com/api",
			TeamSlug:    "",
			ProjectSlug: "",
		}, err
	}
	return ReadConfigFile(path)
}

// DeleteUserConfigFile deletes a user  config file
func DeleteUserConfigFile() error {
	return WriteUserConfigFile(&TurborepoConfig{})
}
