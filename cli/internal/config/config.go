package config

import (
	"fmt"
	"io"
	"io/ioutil"
	"net/url"
	"os"
	"path/filepath"
	"runtime"
	"strings"

	"github.com/vercel/turborepo/cli/internal/client"
	"github.com/vercel/turborepo/cli/internal/fs"

	hclog "github.com/hashicorp/go-hclog"
	"github.com/kelseyhightower/envconfig"
	"github.com/mattn/go-isatty"
	"github.com/mitchellh/cli"
)

const (
	// EnvLogLevel is the environment log level
	EnvLogLevel = "TURBO_LOG_LEVEL"
)

// IsCI returns true if running in a CI/CD environment
func IsCI() bool {
	return !isatty.IsTerminal(os.Stdout.Fd()) || os.Getenv("CI") != ""
}

// Config is a struct that contains user inputs and our logger
type Config struct {
	Logger hclog.Logger
	// TODO: Token through ApiUrl should maybe be grouped together
	// in their own struct, as they will come from config files

	// Bearer token
	Token string
	// vercel.com / remote cache team id
	TeamId string
	// vercel.com / remote cache team slug
	TeamSlug string
	// Backend API URL
	ApiUrl string
	// Login URL
	LoginUrl string
	// Turborepo CLI Version
	TurboVersion string
	Cache        *CacheConfig
	// package.json at the root of the repo
	RootPackageJSON *fs.PackageJSON
	// Current Working Directory
	Cwd fs.AbsolutePath

	UsePreflight      bool
	MaxClientFailures uint64
}

// IsLoggedIn returns true if we have a token and either a team id or team slug
func (c *Config) IsLoggedIn() bool {
	return c.Token != "" && (c.TeamId != "" || c.TeamSlug != "")
}

// CacheConfig
type CacheConfig struct {
	// Number of async workers
	Workers int
}

// ParseAndValidate parses the cmd line flags / env vars, and verifies that all required
// flags have been set. Users can pass in flags when calling a subcommand, or set env vars
// with the prefix 'TURBO_'. If both values are set, the env var value will be used.
func ParseAndValidate(args []string, ui cli.Ui, turboVersion string) (c *Config, err error) {

	// Special check for ./turbo invocation without any args
	// Return the help message
	if len(args) == 0 {
		args = append(args, "--help")
	}

	cmd, inputFlags := args[0], args[1:]
	// Special check for version command
	// command is ./turbo --version
	if len(inputFlags) == 0 && (cmd == "version" || cmd == "--version" || cmd == "-version") {
		return nil, nil
	}

	cwd, err := selectCwd(args)
	if err != nil {
		return nil, err
	}
	// Precedence is flags > env > config > default
	packageJSONPath := cwd.Join("package.json")
	rootPackageJSON, err := fs.ReadPackageJSON(packageJSONPath.ToStringDuringMigration())
	if err != nil {
		return nil, fmt.Errorf("package.json: %w", err)
	}
	userConfig, err := ReadUserConfigFile()
	if err != nil {
		return nil, fmt.Errorf("reading user config file: %v", err)
	}
	if userConfig == nil {
		userConfig = defaultUserConfig()
	}
	partialConfig, err := ReadRepoConfigFile(cwd)
	if err != nil {
		return nil, fmt.Errorf("reading repo config file: %v", err)
	}
	if partialConfig == nil {
		partialConfig = defaultRepoConfig()
	}
	partialConfig.Token = userConfig.Token

	enverr := envconfig.Process("TURBO", partialConfig)
	if enverr != nil {
		return nil, fmt.Errorf("invalid environment variable: %w", err)
	}

	if partialConfig.Token == "" && IsCI() {
		vercelArtifactsToken := os.Getenv("VERCEL_ARTIFACTS_TOKEN")
		vercelArtifactsOwner := os.Getenv("VERCEL_ARTIFACTS_OWNER")
		if vercelArtifactsToken != "" {
			partialConfig.Token = vercelArtifactsToken
		}
		if vercelArtifactsOwner != "" {
			partialConfig.TeamId = vercelArtifactsOwner
		}
	}

	app := args[0]

	// Determine our log level if we have any. First override we check if env var
	level := hclog.NoLevel
	if v := os.Getenv(EnvLogLevel); v != "" {
		level = hclog.LevelFromString(v)
		if level == hclog.NoLevel {
			return nil, fmt.Errorf("%s value %q is not a valid log level", EnvLogLevel, v)
		}
	}

	usePreflight := os.Getenv("TURBO_PREFLIGHT") == "true"

	// Process arguments looking for `-v` flags to control the log level.
	// This overrides whatever the env var set.
	for _, arg := range args {
		if len(arg) != 0 && arg[0] != '-' {
			continue
		}
		switch {
		case arg == "-v":
			if level == hclog.NoLevel || level > hclog.Info {
				level = hclog.Info
			}
		case arg == "-vv":
			if level == hclog.NoLevel || level > hclog.Debug {
				level = hclog.Debug
			}
		case arg == "-vvv":
			if level == hclog.NoLevel || level > hclog.Trace {
				level = hclog.Trace
			}
		case strings.HasPrefix(arg, "--api="):
			apiUrl := arg[len("--api="):]
			if _, err := url.ParseRequestURI(apiUrl); err != nil {
				return nil, fmt.Errorf("%s is an invalid URL", apiUrl)
			}
			partialConfig.ApiUrl = apiUrl
		case strings.HasPrefix(arg, "--url="):
			loginUrl := arg[len("--url="):]
			if _, err := url.ParseRequestURI(loginUrl); err != nil {
				return nil, fmt.Errorf("%s is an invalid URL", loginUrl)
			}
			partialConfig.LoginUrl = loginUrl
		case strings.HasPrefix(arg, "--token="):
			partialConfig.Token = arg[len("--token="):]
		case strings.HasPrefix(arg, "--team="):
			partialConfig.TeamSlug = arg[len("--team="):]
		case arg == "--preflight":
			usePreflight = true
		default:
			continue
		}
	}

	// Default output is nowhere unless we enable logging.
	var output io.Writer = ioutil.Discard
	color := hclog.ColorOff
	if level != hclog.NoLevel {
		output = os.Stderr
		color = hclog.AutoColor
	}

	logger := hclog.New(&hclog.LoggerOptions{
		Name:   app,
		Level:  level,
		Color:  color,
		Output: output,
	})

	maxRemoteFailCount := uint64(3)

	c = &Config{
		Logger:       logger,
		Token:        partialConfig.Token,
		TeamSlug:     partialConfig.TeamSlug,
		TeamId:       partialConfig.TeamId,
		ApiUrl:       partialConfig.ApiUrl,
		LoginUrl:     partialConfig.LoginUrl,
		TurboVersion: turboVersion,
		Cache: &CacheConfig{
			Workers: runtime.NumCPU() + 2,
		},
		RootPackageJSON: rootPackageJSON,
		Cwd:             cwd,

		UsePreflight:      usePreflight,
		MaxClientFailures: maxRemoteFailCount,
	}
	return c, nil
}

// NewClient returns a new ApiClient instance using the values from
// this Config instance.
func (c *Config) NewClient() *client.ApiClient {
	apiClient := client.NewClient(
		c.ApiUrl,
		c.Logger,
		c.TurboVersion,
		c.TeamId,
		c.TeamSlug,
		c.MaxClientFailures,
		c.UsePreflight,
	)
	apiClient.SetToken(c.Token)
	return apiClient
}

// Selects the current working directory from OS
// and overrides with the `--cwd=` input argument
// The various package managers we support resolve symlinks at this stage,
// so we do as well. This means that relative references out of the monorepo
// will be relative to the resolved path, not necessarily the path that the
// user uses to access the monorepo.
func selectCwd(inputArgs []string) (fs.AbsolutePath, error) {
	cwd, err := fs.GetCwd()
	if err != nil {
		return "", err
	}
	for _, arg := range inputArgs {
		if arg == "--" {
			break
		} else if strings.HasPrefix(arg, "--cwd=") {
			if len(arg[len("--cwd="):]) > 0 {
				cwdArgRaw := arg[len("--cwd="):]
				resolved, err := filepath.EvalSymlinks(cwdArgRaw)
				if err != nil {
					return "", err
				}
				cwdArg, err := fs.CheckedToAbsolutePath(resolved)
				if err != nil {
					// the argument is a relative path. Join it with our actual cwd
					return cwd.Join(cwdArgRaw), nil
				}
				return cwdArg, nil
			}
		}
	}
	return cwd, nil
}
