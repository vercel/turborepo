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
	"turbo/internal/client"

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
	// Bearer token
	Token string
	// Turborepo.com team id
	TeamId string
	// Turborepo.com team id
	TeamSlug string
	// Backend API URL
	ApiUrl string
	// Login URL
	LoginUrl string
	// Backend retryable http client
	ApiClient *client.ApiClient
	// Turborepo CLI Version
	TurboVersion string
	Cache        *CacheConfig
}

// CacheConfig
type CacheConfig struct {
	// Number of async workers
	Workers int
	// Cache directory
	Dir string
	// HTTP URI of the cache
	Url string
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

	// Pop the subcommand into 'cmd'
	// flags.Parse does not work when the subcommand is included
	cmd, inputFlags := args[0], args[1:]

	// Special check for help commands
	// command is ./turbo --help or --version
	if len(inputFlags) == 0 && (cmd == "help" || cmd == "--help" || cmd == "-help" || cmd == "version" || cmd == "--version" || cmd == "-version") {
		return nil, nil
	}
	// command is ./turbo $subcommand --help
	if len(inputFlags) == 1 && (inputFlags[0] == "help" || inputFlags[0] == "--help" || inputFlags[0] == "-help") {
		return nil, nil
	}
	// Precendence is flags > env > config > default
	userConfig, err := ReadUserConfigFile()
	if err != nil {
		// not logged in
	}
	partialConfig, err := ReadConfigFile(filepath.Join(".turbo", "config.json"))
	if err != nil {
		// not linked
	}
	partialConfig.Token = userConfig.Token

	enverr := envconfig.Process("TURBO", partialConfig)
	fmt.Printf("TURBO_TEAM: %v, %v\n", partialConfig.TeamSlug, os.Getenv("TURBO_TEAM"))
	if enverr != nil {
		return nil, fmt.Errorf("invalid environment variable: %w", err)
	}

	if partialConfig.Token == "" && IsCI() {
		partialConfig.Token = os.Getenv("VERCEL_ARTIFACTS_TOKEN")
		partialConfig.TeamId = os.Getenv("VERCEL_ARTIFACTS_OWNER")
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

	shouldEnsureTeam := false
	// Process arguments looking for `-v` flags to control the log level.
	// This overrides whatever the env var set.
	var outArgs []string
	for _, arg := range args {
		if len(arg) != 0 && arg[0] != '-' {
			outArgs = append(outArgs, arg)
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
			shouldEnsureTeam = true
		default:
			outArgs = append(outArgs, arg)
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

	apiClient := client.NewClient(partialConfig.ApiUrl, logger, turboVersion)

	c = &Config{
		Logger:       logger,
		Token:        partialConfig.Token,
		TeamSlug:     partialConfig.TeamSlug,
		TeamId:       partialConfig.TeamId,
		ApiUrl:       partialConfig.ApiUrl,
		LoginUrl:     partialConfig.LoginUrl,
		ApiClient:    apiClient,
		TurboVersion: turboVersion,
		Cache: &CacheConfig{
			Workers: runtime.NumCPU() + 2,
			Dir:     filepath.Join("node_modules", ".cache", "turbo"),
		},
	}

	c.ApiClient.SetToken(partialConfig.Token)

	if shouldEnsureTeam || partialConfig.TeamSlug != "" {
		if err := c.ensureTeam(); err != nil {
			return c, err
		}
	}

	return c, nil
}

func (c *Config) ensureTeam() error {
	// if c.Token == "" {
	// 	if IsCI() {
	// 		fmt.Println(ui.Warn("no token has been provided, but you specified a Turborepo team and project. If this is intended (e.g. a pull request on an open source GitHub project from an outside contributor triggered this), you can ignore this warning. Otherwise, please run `turbo login`, pass `--token` flag, or set `TURBO_TOKEN` environment variable to enable remote caching. In the meantime, turbo will attempt to continue with local caching."))
	// 		return nil
	// 	}
	// 	return fmt.Errorf("no credentials found. Please run `turbo login`, pass `--token` flag, or set TURBO_TOKEN environment variable")
	// }
	// req, err := graphql.NewGetTeamRequest(c.ApiUrl, &graphql.GetTeamVariables{
	// 	Slug: (*graphql.String)(&c.TeamSlug),
	// })
	// if err != nil {
	// 	return fmt.Errorf("could not fetch team information: %w", err)
	// }
	// req.Header.Set("Authorization", "Bearer "+c.Token)
	// res, resErr := req.Execute(c.GraphQLClient.Client)
	// if resErr != nil {
	// 	return fmt.Errorf("could not fetch team information: %w", resErr)
	// }

	// if res.Team.ID == "" {
	// 	return fmt.Errorf("could not fetch team information. Check the spelling of `%v` and make sure that the %v team exists on turborepo.com and that you have access to it", c.TeamSlug, c.TeamSlug)
	// }

	// c.TeamId = res.Team.ID
	// c.TeamSlug = res.Team.Slug

	return nil
}

// IsLogged returns true if the user is logged into turborepo.com
func (c *Config) IsLoggedIn() bool {
	return c.Token != ""
}

// IsTurborepoLinked returns true if the project is linked (or has enough info to make API requests)
func (c *Config) IsTurborepoLinked() bool {
	return (c.TeamId != "" || c.TeamSlug != "")
}
