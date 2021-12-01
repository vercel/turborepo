package login

import (
	"fmt"
	"strings"
	"turbo/internal/config"

	"github.com/fatih/color"
	"github.com/mitchellh/cli"
)

// LoginCommand is a Command implementation that tells Turbo to run a task
type LoginCommand struct {
	Config *config.Config
	Ui     *cli.ColoredUi
}

// Synopsis of run command
func (c *LoginCommand) Synopsis() string {
	return "DEPRECATED - Login to your Turborepo.com account"
}

// Help returns information about the `run` command
func (c *LoginCommand) Help() string {
	helpText := `
Usage: turbo login

  Login to your Turborepo.com account
`
	return strings.TrimSpace(helpText)
}

// Run executes tasks in the monorepo
func (c *LoginCommand) Run(args []string) int {
	pref := color.New(color.Bold, color.FgRed, color.ReverseVideo).Sprint(" ERROR ")
	c.Ui.Output(fmt.Sprintf("%s%s", pref, color.RedString(" This command has been deprecated. Please use `turbo link` instead.")))
	return 1
}
