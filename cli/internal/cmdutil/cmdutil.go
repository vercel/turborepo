package cmdutil

import (
	"fmt"
	"io"
	"io/ioutil"
	"os"
	"runtime/debug"

	"github.com/hashicorp/go-hclog"
	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/client"
	"github.com/vercel/turborepo/cli/internal/config"
	tlogger "github.com/vercel/turborepo/cli/internal/logger"
	"github.com/vercel/turborepo/cli/internal/process"
)

const (
	// EnvLogLevel is the environment log level
	EnvLogLevel = "TURBO_LOG_LEVEL"
)

type Helper struct {
	Config    *config.Config
	Logger    *tlogger.Logger
	Processes *process.Manager
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
		// To view a CPU trace, use "go tool trace [file]". Note that the trace
		// viewer doesn't work under Windows Subsystem for Linux for some reason.
		if h.Config.Trace != "" {
			if done := createTraceFile(args, h.Config.Trace); done == nil {
				os.Exit(1)
			} else {
				defer done()
			}
		}

		// To view a heap trace, use "go tool pprof [file]" and type "top". You can
		// also drop it into https://speedscope.app and use the "left heavy" or
		// "sandwich" view modes.
		if h.Config.Heap != "" {
			if done := createHeapFile(args, h.Config.Heap); done == nil {
				os.Exit(1)
			} else {
				defer done()
			}
		}

		// To view a CPU profile, drop the file into https://speedscope.app.
		// Note: Running the CPU profiler doesn't work under Windows subsystem for
		// Linux. The profiler has to be built for native Windows and run using the
		// command prompt instead.
		if h.Config.CpuProfile != "" {
			if done := createCpuprofileFile(args, h.Config.CpuProfile); done == nil {
				os.Exit(1)
			} else {
				defer done()
			}
		}

		if h.Config.CpuProfile == "" {
			// Disable the GC since we're just going to allocate a bunch of memory
			// and then exit anyway. This speedup is not insignificant. Make sure to
			// only do this here once we know that we're not going to be a long-lived
			// process though.
			if !h.Config.NoGC {
				debug.SetGCPercent(-1)
			}
		}

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
