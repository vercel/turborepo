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
	"github.com/vercel/turborepo/cli/internal/turbopath"

	hclog "github.com/hashicorp/go-hclog"
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
	// Turborepo CLI Version
	TurboVersion string
	Cache        *CacheConfig
	// Current Working Directory
	Cwd turbopath.AbsolutePath

	UsePreflight      bool
	MaxClientFailures uint64

	LoginURL string

	UserConfig   *UserConfig
	RepoConfig   *RepoConfig
	RemoteConfig client.RemoteConfig
}

// CacheConfig
type CacheConfig struct {
	// Number of async workers
	Workers int
}

// ParseAndValidate parses the cmd line flags / env vars, and verifies that all required
// flags have been set. Users can pass in flags when calling a subcommand, or set env vars
// with the prefix 'TURBO_'. If both values are set, the env var value will be used.
func ParseAndValidate(args []string, ui cli.Ui, turboVersion string, userConfigFile turbopath.AbsolutePath) (c *Config, err error) {

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
	userConfig, err := ReadUserConfigFile(userConfigFile)
	if err != nil {
		return nil, fmt.Errorf("reading user config file: %v", err)
	}
	token := userConfig.Token()
	repoConfig, err := ReadRepoConfigFile(GetRepoConfigPath(cwd))
	if err != nil {
		return nil, fmt.Errorf("reading repo config file: %v", err)
	}
	remoteConfig := repoConfig.GetRemoteConfig(token)

	if token == "" && IsCI() {
		vercelArtifactsToken := os.Getenv("VERCEL_ARTIFACTS_TOKEN")
		vercelArtifactsOwner := os.Getenv("VERCEL_ARTIFACTS_OWNER")
		if vercelArtifactsToken != "" {
			remoteConfig.Token = vercelArtifactsToken
		}
		if vercelArtifactsOwner != "" {
			//repoConfig.TeamId = vercelArtifactsOwner
			remoteConfig.TeamID = vercelArtifactsOwner
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

	loginURL := repoConfig.LoginURL()
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
			apiURL := arg[len("--api="):]
			if _, err := url.ParseRequestURI(apiURL); err != nil {
				return nil, fmt.Errorf("%s is an invalid URL", apiURL)
			}
			remoteConfig.APIURL = apiURL
		case strings.HasPrefix(arg, "--url="):
			loginURLArg := arg[len("--url="):]
			if _, err := url.ParseRequestURI(loginURLArg); err != nil {
				return nil, fmt.Errorf("%s is an invalid URL", loginURLArg)
			}
			loginURL = loginURLArg
		case strings.HasPrefix(arg, "--token="):
			remoteConfig.Token = arg[len("--token="):]
		case strings.HasPrefix(arg, "--team="):
			remoteConfig.TeamSlug = arg[len("--team="):]
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
		UserConfig:   userConfig,
		RepoConfig:   repoConfig,
		RemoteConfig: remoteConfig,
		LoginURL:     loginURL,
		TurboVersion: turboVersion,
		Cache: &CacheConfig{
			Workers: runtime.NumCPU() + 2,
		},
		Cwd: cwd,

		UsePreflight:      usePreflight,
		MaxClientFailures: maxRemoteFailCount,
	}
	return c, nil
}

// NewClient returns a new ApiClient instance using the values from
// this Config instance.
func (c *Config) NewClient() *client.ApiClient {
	apiClient := client.NewClient(
		c.RemoteConfig,
		c.Logger,
		c.TurboVersion,
		client.Opts{UsePreflight: c.UsePreflight},
	)
	return apiClient
}

// Selects the current working directory from OS
// and overrides with the `--cwd=` input argument
// The various package managers we support resolve symlinks at this stage,
// so we do as well. This means that relative references out of the monorepo
// will be relative to the resolved path, not necessarily the path that the
// user uses to access the monorepo.
func selectCwd(inputArgs []string) (turbopath.AbsolutePath, error) {
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
