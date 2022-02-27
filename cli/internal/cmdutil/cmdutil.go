package cmdutil

import (
	"fmt"

	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/logger"
)

type Helper struct {
	debug  *bool
	Config *config.Config
	Logger *logger.Logger
}

func (h *Helper) Debug() bool {
	return *h.debug
}

func (h *Helper) SetDebug(debug *bool) {
	h.debug = debug
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
