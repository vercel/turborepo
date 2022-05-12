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

// UnlinkCommand is a Command implementation allows the user to login to turbo
type UnlinkCommand struct {
	Config *config.Config
	Ui     *cli.ColoredUi
}

// Synopsis of run command
func (c *UnlinkCommand) Synopsis() string {
	return "Unlink the current directory from your Vercel organization and disable Remote Caching (beta)."
}

// Help returns information about the `run` command
func (c *UnlinkCommand) Help() string {
	helpText := `
Usage: turbo unlink

    Unlink the current directory from your Vercel organization and disable Remote Caching (beta).
`
	return strings.TrimSpace(helpText)
}

// Run executes tasks in the monorepo
func (c *UnlinkCommand) Run(args []string) int {
	if err := config.WriteRepoConfigFile(c.Config.Fs, c.Config.Cwd, &config.TurborepoConfig{}); err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("could not unlink. Something went wrong: %w", err))
		return 1
	}
	c.Ui.Output(util.Sprintf("${GREY}> Disabled Remote Caching${RESET}"))
	return 0
}

// logError logs an error and outputs it to the UI.
func (c *UnlinkCommand) logError(log hclog.Logger, prefix string, err error) {
	log.Error(prefix, "error", err)

	if prefix != "" {
		prefix += ": "
	}

	c.Ui.Error(fmt.Sprintf("%s%s%s", ui.ERROR_PREFIX, prefix, color.RedString(" %v", err)))
}
