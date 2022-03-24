package cmdutil

import (
	"fmt"

	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/process"
	"github.com/vercel/turborepo/cli/internal/ui/variants"
)

const (
	// EnvLogLevel is the environment log level
	EnvLogLevel = "TURBO_LOG_LEVEL"
)

type Helper struct {
	Config    *config.Config
	Ui        *variants.Default
	Processes *process.Manager
}

func (h *Helper) LogWarning(format string, args ...interface{}) error {
	err := fmt.Errorf(format, args...)
	h.Config.Logger.Warn("warning", err)
	return h.Ui.Errorf(err.Error())
}

func (h *Helper) LogError(format string, args ...interface{}) error {
	err := fmt.Errorf(format, args...)
	h.Config.Logger.Error("error", err)
	return h.Ui.Errorf(err.Error())
}
