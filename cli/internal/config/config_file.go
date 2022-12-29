package config

import (
	"os"

	"github.com/spf13/pflag"
	"github.com/spf13/viper"
	"github.com/vercel/turbo/cli/internal/client"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// CLIConfigProvider is an interface for providing configuration values from the CLI
// It can be implemented by either a pflag.FlagSet struct or a turbostate.Args struct.
type CLIConfigProvider interface {
	GetColor() bool
	GetNoColor() bool
	GetLogin() (string, error)
	GetAPI() (string, error)
	GetTeam() (string, error)
	GetToken() (string, error)
	GetCwd() (string, error)
}

// FlagSet is a wrapper so that the CLIConfigProvider interface can be implemented
// on pflag.FlagSet.
type FlagSet struct {
	*pflag.FlagSet
}

// GetColor returns the value of the `color` flag. Used to implement CLIConfigProvider interface.
func (p FlagSet) GetColor() bool {
	return p.Changed("color")
}

// GetNoColor returns the value of the `no-color` flag. Used to implement CLIConfigProvider interface.
func (p FlagSet) GetNoColor() bool {
	return p.Changed("no-color")
}

// GetLogin returns the value of the `login` flag. Used to implement CLIConfigProvider interface.
func (p FlagSet) GetLogin() (string, error) {
	return p.GetString("login")
}

// GetAPI returns the value of the `api` flag. Used to implement CLIConfigProvider interface.
func (p FlagSet) GetAPI() (string, error) {
	return p.GetString("api")
}

// GetTeam returns the value of the `team` flag. Used to implement CLIConfigProvider interface.
func (p FlagSet) GetTeam() (string, error) {
	return p.GetString("team")
}

// GetToken returns the value of the `token` flag. Used to implement CLIConfigProvider interface.
func (p FlagSet) GetToken() (string, error) {
	return p.GetString("token")
}

// GetCwd returns the value of the `cwd` flag. Used to implement CLIConfigProvider interface.
func (p FlagSet) GetCwd() (string, error) {
	return p.GetString("cwd")
}

// RepoConfig is a configuration object for the logged-in turborepo.com user
type RepoConfig struct {
	repoViper *viper.Viper
	path      turbopath.AbsoluteSystemPath
}

// LoginURL returns the configured URL for authenticating the user
func (rc *RepoConfig) LoginURL() string {
	return rc.repoViper.GetString("loginurl")
}

// SetTeamID sets the teamID and clears the slug, since it may have been from an old team
func (rc *RepoConfig) SetTeamID(teamID string) error {
	// Note that we can't use viper.Set to set a nil value, we have to merge it in
	newVals := map[string]interface{}{
		"teamid":   teamID,
		"teamslug": nil,
	}
	if err := rc.repoViper.MergeConfigMap(newVals); err != nil {
		return err
	}
	return rc.write()
}

// GetRemoteConfig produces the necessary values for an API client configuration
func (rc *RepoConfig) GetRemoteConfig(token string) client.RemoteConfig {
	return client.RemoteConfig{
		Token:    token,
		TeamID:   rc.repoViper.GetString("teamid"),
		TeamSlug: rc.repoViper.GetString("teamslug"),
		APIURL:   rc.repoViper.GetString("apiurl"),
	}
}

// Internal call to save this config data to the user config file.
func (rc *RepoConfig) write() error {
	if err := rc.path.EnsureDir(); err != nil {
		return err
	}
	return rc.repoViper.WriteConfig()
}

// Delete deletes the config file. This repo config shouldn't be used
// afterwards, it needs to be re-initialized
func (rc *RepoConfig) Delete() error {
	return rc.path.Remove()
}

// UserConfig is a wrapper around the user-specific configuration values
// for Turborepo.
type UserConfig struct {
	userViper *viper.Viper
	path      turbopath.AbsoluteSystemPath
}

// Token returns the Bearer token for this user if it exists
func (uc *UserConfig) Token() string {
	return uc.userViper.GetString("token")
}

// SetToken saves a Bearer token for this user, writing it to the
// user config file, creating it if necessary
func (uc *UserConfig) SetToken(token string) error {
	// Technically Set works here, due to how overrides work, but use merge for consistency
	if err := uc.userViper.MergeConfigMap(map[string]interface{}{"token": token}); err != nil {
		return err
	}
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

// ReadUserConfigFile creates a UserConfig using the
// specified path as the user config file. Note that the path or its parents
// do not need to exist. On a write to this configuration, they will be created.
func ReadUserConfigFile(path turbopath.AbsoluteSystemPath, cliConfig CLIConfigProvider) (*UserConfig, error) {
	userViper := viper.New()
	userViper.SetConfigFile(path.ToString())
	userViper.SetConfigType("json")
	userViper.SetEnvPrefix("turbo")
	userViper.MustBindEnv("token")

	token, err := cliConfig.GetToken()
	if err != nil {
		return nil, err
	}
	if token != "" {
		userViper.Set("token", token)
	}

	if err := userViper.ReadInConfig(); err != nil && !os.IsNotExist(err) {
		return nil, err
	}
	return &UserConfig{
		userViper: userViper,
		path:      path,
	}, nil
}

// AddUserConfigFlags adds per-user configuration item flags to the given flagset
func AddUserConfigFlags(flags *pflag.FlagSet) {
	flags.String("token", "", "Set the auth token for API calls")
}

// DefaultUserConfigPath returns the default platform-dependent place that
// we store the user-specific configuration.
func DefaultUserConfigPath() turbopath.AbsoluteSystemPath {
	return fs.GetUserConfigDir().UntypedJoin("config.json")
}

const (
	_defaultAPIURL   = "https://vercel.com/api"
	_defaultLoginURL = "https://vercel.com"
)

// ReadRepoConfigFile creates a RepoConfig using the
// specified path as the repo config file. Note that the path or its
// parents do not need to exist. On a write to this configuration, they
// will be created.
func ReadRepoConfigFile(path turbopath.AbsoluteSystemPath, cliConfig CLIConfigProvider) (*RepoConfig, error) {
	repoViper := viper.New()
	repoViper.SetConfigFile(path.ToString())
	repoViper.SetConfigType("json")
	repoViper.SetEnvPrefix("turbo")
	repoViper.MustBindEnv("apiurl", "TURBO_API")
	repoViper.MustBindEnv("loginurl", "TURBO_LOGIN")
	repoViper.MustBindEnv("teamslug", "TURBO_TEAM")
	repoViper.MustBindEnv("teamid")
	repoViper.SetDefault("apiurl", _defaultAPIURL)
	repoViper.SetDefault("loginurl", _defaultLoginURL)

	login, err := cliConfig.GetLogin()
	if err != nil {
		return nil, err
	}
	if login != "" {
		repoViper.Set("loginurl", login)
	}

	api, err := cliConfig.GetAPI()
	if err != nil {
		return nil, err
	}
	if api != "" {
		repoViper.Set("apiurl", api)
	}

	team, err := cliConfig.GetTeam()
	if err != nil {
		return nil, err
	}
	if team != "" {
		repoViper.Set("teamslug", team)
	}

	if err := repoViper.ReadInConfig(); err != nil && !os.IsNotExist(err) {
		return nil, err
	}
	// If team was set via commandline, don't read the teamId from the config file, as it
	// won't necessarily match.
	if team != "" {
		repoViper.Set("teamid", "")
	}
	return &RepoConfig{
		repoViper: repoViper,
		path:      path,
	}, nil
}

// AddRepoConfigFlags adds per-repository configuration items to the given flagset
func AddRepoConfigFlags(flags *pflag.FlagSet) {
	flags.String("team", "", "Set the team slug for API calls")
	flags.String("api", "", "Override the endpoint for API calls")
	flags.String("login", "", "Override the login endpoint")
}

// GetRepoConfigPath reads the user-specific configuration values
func GetRepoConfigPath(repoRoot turbopath.AbsoluteSystemPath) turbopath.AbsoluteSystemPath {
	return repoRoot.UntypedJoin(".turbo", "config.json")
}
