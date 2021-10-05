package login

import (
	"fmt"
	"strings"
	"turbo/internal/config"
	"turbo/internal/ui"

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
	return "Logout of your Turborepo account"
}

// Help returns information about the `run` command
func (c *LogoutCommand) Help() string {
	helpText := `
Usage: turbo logout

    Login of your Turborepo account
`
	return strings.TrimSpace(helpText)
}

// Run executes tasks in the monorepo
func (c *LogoutCommand) Run(args []string) int {
	if err := config.DeleteUserConfigFile(); err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("Could not logout. Something went wrong: %w", err))
		return 1
	}
	c.Ui.Output(ui.Dim("Logged out"))
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
