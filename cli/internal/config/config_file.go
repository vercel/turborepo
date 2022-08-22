package config

import (
	"encoding/json"
	"errors"
	"os"

	"github.com/spf13/viper"
	"github.com/vercel/turborepo/cli/internal/fs"
)

// TurborepoConfig is a configuration object for the logged-in turborepo.com user
type TurborepoConfig struct {
	// Token is a bearer token
	// TODO: this should be dropped, it's a per-user config item, not per-repo,
	// and should be properly managed by Viper
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

// UserConfig is a wrapper around the user-specific configuration values
// for Turborepo.
type UserConfig struct {
	userViper *viper.Viper
	path      fs.AbsolutePath
}

// Token returns the Bearer token for this user if it exists
func (uc *UserConfig) Token() string {
	return uc.userViper.GetString("token")
}

// SetToken saves a Bearer token for this user, writing it to the
// user config file, creating it if necessary
func (uc *UserConfig) SetToken(token string) error {
	uc.userViper.Set("token", token)
	return uc.write()
}

// Internal call to save this config data to the user config file.
func (uc *UserConfig) write() error {
	if err := uc.path.EnsureDir(); err != nil {
		return err
	}
	return uc.userViper.WriteConfig()
}

// Delete deletes the config file. This user config shouldn't be used
// afterwards, it needs to be re-initialized
func (uc *UserConfig) Delete() error {
	return uc.path.Remove()
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

// ReadUserConfigFile is public for tests, it creates a UserConfig using the
// specified path as the user config file. Note that the path or its parents
// do not need to exist. On a write to this configuration, they will be created.
func ReadUserConfigFile(path fs.AbsolutePath) (*UserConfig, error) {
	userViper := viper.New()
	userViper.SetConfigFile(path.ToString())
	userViper.SetConfigType("json")
	if err := userViper.ReadInConfig(); err != nil && !os.IsNotExist(err) {
		return nil, err
	}
	return &UserConfig{
		userViper: userViper,
		path:      path,
	}, nil
}

func getUserConfigPath() fs.AbsolutePath {
	return fs.GetUserConfigDir().Join("config.json")
}

// GetUserConfig reads a user config file
func GetUserConfig() (*UserConfig, error) {
	return ReadUserConfigFile(getUserConfigPath())
}

// ReadRepoConfigFile reads the user-specific configuration values
func ReadRepoConfigFile(repoRoot fs.AbsolutePath) (*TurborepoConfig, error) {
	path := repoRoot.Join(".turbo", "config.json")
	return readConfigFile(path, defaultRepoConfig)
}
