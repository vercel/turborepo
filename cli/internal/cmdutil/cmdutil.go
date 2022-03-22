package cmdutil

import (
	"fmt"

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
