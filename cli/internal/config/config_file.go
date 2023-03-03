package config

import (
	"os"

	"github.com/spf13/viper"
	"github.com/vercel/turbo/cli/internal/client"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/turbostate"
)

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
func ReadUserConfigFile(path turbopath.AbsoluteSystemPath, cliConfig *turbostate.ParsedArgsFromRust) (*UserConfig, error) {
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
func ReadRepoConfigFile(path turbopath.AbsoluteSystemPath, cliConfig *turbostate.ParsedArgsFromRust) (*RepoConfig, error) {
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

// GetRepoConfigPath reads the user-specific configuration values
func GetRepoConfigPath(repoRoot turbopath.AbsoluteSystemPath) turbopath.AbsoluteSystemPath {
	return repoRoot.UntypedJoin(".turbo", "config.json")
}
