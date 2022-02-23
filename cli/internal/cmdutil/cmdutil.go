package cmdutil

import (
	"github.com/vercel/turborepo/cli/internal/logger"
)

type Helper struct {
	debug  *bool
	Logger *logger.Logger
}

func (h *Helper) Debug() bool {
	return *h.debug
}

func (h *Helper) SetDebug(debug *bool) {
	h.debug = debug
}
