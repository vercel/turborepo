package cmdutil

import (
	"fmt"
	"io"
	"io/ioutil"
	"os"

	"github.com/hashicorp/go-hclog"
	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/client"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/logger"
)

const (
	// EnvLogLevel is the environment log level
	EnvLogLevel = "TURBO_LOG_LEVEL"
)

type Helper struct {
	Config *config.Config
	Logger *logger.Logger
}

func (h *Helper) LogWarning(format string, args ...interface{}) error {
	err := fmt.Errorf(format, args...)
	h.Config.Logger.Warn("warning", err)
	return h.Logger.Errorf(err.Error())
}

func (h *Helper) LogError(format string, args ...interface{}) error {
	err := fmt.Errorf(format, args...)
	h.Config.Logger.Error("error", err)
	return h.Logger.Errorf(err.Error())
}

func (h *Helper) PreRun() func(cmd *cobra.Command, args []string) error {
	return func(cmd *cobra.Command, args []string) error {
		if !h.Config.NoColor {
			os.Setenv("FORCE_COLOR", "1")
		}

		// Determine our log level if we have any. First override we check if env var
		level := hclog.NoLevel
		if v := os.Getenv(EnvLogLevel); v != "" {
			level = hclog.LevelFromString(v)
			if level == hclog.NoLevel {
				return h.Logger.Errorf("%s value %q is not a valid log level", EnvLogLevel, v)
			}
		}

		// Process arguments looking for `-v` flags to control the log level.
		// This overrides whatever the env var set.
		switch {
		case h.Config.Level == 1:
			if level == hclog.NoLevel || level > hclog.Info {
				level = hclog.Info
			}
		case h.Config.Level == 2:
			if level == hclog.NoLevel || level > hclog.Debug {
				level = hclog.Debug
			}
		case h.Config.Level == 3:
			if level == hclog.NoLevel || level > hclog.Trace {
				level = hclog.Trace
			}
		default:
		}

		// Default output is nowhere unless we enable logging.
		var output io.Writer = ioutil.Discard
		color := hclog.ColorOff
		if level != hclog.NoLevel {
			output = os.Stderr
			color = hclog.AutoColor
		}

		hlogger := hclog.New(&hclog.LoggerOptions{
			Name:   cmd.Name(),
			Level:  level,
			Color:  color,
			Output: output,
		})

		maxRemoteFailCount := 3
		apiClient := client.NewClient(h.Config.ApiUrl, hlogger, h.Config.Version, h.Config.TeamId, h.Config.TeamSlug, uint64(maxRemoteFailCount))

		h.Config.Logger = hlogger
		h.Config.ApiClient = apiClient

		h.Config.ApiClient.SetToken(h.Config.Token)

		return nil
	}
}
