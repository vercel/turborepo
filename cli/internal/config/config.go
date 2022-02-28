package config

import (
	"os"
	"path/filepath"
	"runtime"

	"github.com/hashicorp/go-hclog"
	"github.com/kelseyhightower/envconfig"
	"github.com/mattn/go-isatty"
	"github.com/vercel/turborepo/cli/internal/client"
	"github.com/vercel/turborepo/cli/internal/logger"
)

func IsCI() bool {
	return !isatty.IsTerminal(os.Stdout.Fd()) || os.Getenv("CI") != ""
}

type Config struct {
	Level      int
	Token      string
	TeamId     string
	TeamSlug   string
	ApiUrl     string
	LoginUrl   string
	Version    string
	NoColor    bool
	Logger     hclog.Logger
	ApiClient  *client.ApiClient
	Cache      *CacheConfig
}

type CacheConfig struct {
	Workers int
	Dir     string
}

func New(logger *logger.Logger, version string) (*Config, error) {
	userConfig, _ := ReadUserConfigFile()
	partialConfig, _ := ReadConfigFile(filepath.Join(".turbo", "config.json"))
	partialConfig.Token = userConfig.Token

	enverr := envconfig.Process("TURBO", partialConfig)
	if enverr != nil {
		return nil, logger.Errorf("invalid environment variable: %w", enverr)
	}

	if partialConfig.Token == "" && IsCI() {
		partialConfig.Token = os.Getenv("VERCEL_ARTIFACTS_TOKEN")
		partialConfig.TeamId = os.Getenv("VERCEL_ARTIFACTS_OWNER")
	}

	cfg := &Config{
		Token:      partialConfig.Token,
		TeamSlug:   partialConfig.TeamSlug,
		TeamId:     partialConfig.TeamId,
		ApiUrl:     partialConfig.ApiUrl,
		LoginUrl:   partialConfig.LoginUrl,
		Version:    version,
		Cache: &CacheConfig{
			Workers: runtime.NumCPU() + 2,
			Dir:     filepath.Join("node_modules", ".cache", "turbo"),
		},
	}

	return cfg, nil
}

func (c *Config) IsAuthenticated() bool {
	return c.Token != "" && (c.TeamId != "" || c.TeamSlug != "")
}
