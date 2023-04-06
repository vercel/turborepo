package config

import (
	"github.com/spf13/viper"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
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

// DefaultUserConfigPath returns the default platform-dependent place that
// we store the user-specific configuration.
func DefaultUserConfigPath() turbopath.AbsoluteSystemPath {
	return fs.GetUserConfigDir().UntypedJoin("config.json")
}
