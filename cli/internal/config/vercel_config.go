package config

import (
	"encoding/json"
	"fmt"
	"io/ioutil"
	"path/filepath"
	"turbo/internal/fs"

	"github.com/adrg/xdg"
)

// VercelConfig represents the global Vercel configuration
type VercelConfig struct {
	// Team is the vercel.com current team ID
	TeamId string `json:"currentTeam,omitempty"`
	// Collect metrics
	CollectMetrics bool `json:"collectMetrics,omitempty"`
	// API url
	Api string `json:"api,omitempty"`
}

// VercelAuthConfig represents the global Vercel configuration
type VercelAuthConfig struct {
	// Token is the bearer token used to authenticate with the API
	Token string `json:"token,omitempty"`
}

// GetVercelConfig reads the vercel config file from the user's global configuration file
func GetVercelConfig(customConfigPath string) (*VercelConfig, error) {
	config := VercelConfig{}

	if customConfigPath == "" {
		configPath, err := getConfigFilePath("config.json")
		if err != nil {
			return &config, err
		}
		customConfigPath = configPath
	} else {
		customConfigPath, err := filepath.Abs(customConfigPath)
		if err != nil {
			return &config, fmt.Errorf("failed to construct absolute path for %s", customConfigPath)
		}
	}

	b, err := ioutil.ReadFile(customConfigPath)
	if err != nil {
		return &config, err
	}
	if jsonErr := json.Unmarshal(b, &config); jsonErr != nil {
		return &config, jsonErr
	}
	return &config, nil
}

// GetVercelAuthConfig reads the vercel config file from the user's global configuration file
func GetVercelAuthConfig(customConfigPath string) (*VercelAuthConfig, error) {
	config := VercelAuthConfig{}

	if customConfigPath == "" {
		configPath, err := getConfigFilePath("auth.json")
		if err != nil {
			return &config, err
		}
		customConfigPath = configPath
	} else {
		customConfigPath, err := filepath.Abs(customConfigPath)
		if err != nil {
			return &config, fmt.Errorf("failed to construct absolute path for %s", customConfigPath)
		}
	}

	b, err := ioutil.ReadFile(customConfigPath)
	if err != nil {
		return &config, err
	}
	if jsonErr := json.Unmarshal(b, &config); jsonErr != nil {
		return &config, jsonErr
	}
	return &config, nil
}

// getConfigFilePath is a bad attempt at porting this logic out of the vercel cli into Go
// @see https://github.com/vercel/vercel/blob/f18bca97187d17c050695a7a348b8ae02c244ce9/packages/cli/src/util/config/global-path.ts#L18
// for the original implementation. It tries to search find and then respect legacy
// configuration directories
func getConfigFilePath(filename string) (string, error) {
	if vcDataDir, e := xdg.SearchDataFile(filepath.Join("com.vercel.cli", filename)); e != nil {
		tempDir := filepath.Join(xdg.Home, ".now", filename)
		if fs.IsDirectory(tempDir) {
			return tempDir, nil
		} else {
			if nowDataDir, f := xdg.SearchDataFile(filepath.Join("now", filename)); f != nil {
				return "", fmt.Errorf("config file %s found. Please login with `vercel login`", filename)
			} else {
				return nowDataDir, nil
			}
		}
	} else {
		return vcDataDir, nil
	}
}
