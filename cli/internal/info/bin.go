package info

import (
	"fmt"
	"os"
	"strings"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/ui"

	"github.com/fatih/color"
	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
)

type BinCommand struct {
	Config *config.Config
	Ui     *cli.ColoredUi
}

// Synopsis of run command
func (c *BinCommand) Synopsis() string {
	return "Get the path to the Turbo binary"
}

// Help returns information about the `bin` command
func (c *BinCommand) Help() string {
	helpText := `
Usage: turbo bin

  Get the path to the Turbo binary
`
	return strings.TrimSpace(helpText)
}

func (c *BinCommand) Run(args []string) int {
	path, err := os.Executable()
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("could not get path to turbo binary: %w", err))
		return 1
	}
	c.Ui.Output(path)
	return 0
}

// logError logs an error and outputs it to the UI.
func (c *BinCommand) logError(log hclog.Logger, prefix string, err error) {
	log.Error(prefix, "error", err)

	if prefix != "" {
		prefix += ": "
	}

	c.Ui.Error(fmt.Sprintf("%s%s%s", ui.ERROR_PREFIX, prefix, color.RedString(" %v", err)))
}
