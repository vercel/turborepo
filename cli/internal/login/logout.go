package login

import (
	"fmt"
	"strings"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"

	"github.com/fatih/color"
	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
)

// LogoutCommand is a Command implementation allows the user to login to turbo
type LogoutCommand struct {
	Config *config.Config
	Ui     *cli.ColoredUi
}

// Synopsis of run command
func (c *LogoutCommand) Synopsis() string {
	return "Logout of your Vercel account"
}

// Help returns information about the `run` command
func (c *LogoutCommand) Help() string {
	helpText := `
Usage: turbo logout

    Logout of your Vercel account
`
	return strings.TrimSpace(helpText)
}

// Run executes tasks in the monorepo
func (c *LogoutCommand) Run(args []string) int {
	if err := config.DeleteUserConfigFile(); err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("could not logout. Something went wrong: %w", err))
		return 1
	}
	c.Ui.Info(util.Sprintf("${GREY}>>> Logged out${RESET}"))
	return 0
}

// logError logs an error and outputs it to the UI.
func (c *LogoutCommand) logError(log hclog.Logger, prefix string, err error) {
	log.Error(prefix, "error", err)

	if prefix != "" {
		prefix += ": "
	}

	c.Ui.Error(fmt.Sprintf("%s%s%s", ui.ERROR_PREFIX, prefix, color.RedString(" %v", err)))
}
