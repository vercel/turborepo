package info

import (
	"fmt"
	"strings"
	"turbo/internal/config"

	"github.com/fatih/color"
	"github.com/mitchellh/cli"
)

// GraphCommand is a Command implementation that tells Turbo to run a task
type GraphCommand struct {
	Config *config.Config
	Ui     *cli.ColoredUi
}

// Synopsis of run command
func (c *GraphCommand) Synopsis() string {
	return "DEPRECATED - Generate a Dot Graph of your monorepo"
}

// Help returns information about the `run` command
func (c *GraphCommand) Help() string {
	helpText := `
Usage: turbo graph

  Generate a Dot Graph of your monorepo
`
	return strings.TrimSpace(helpText)
}

// Run executes tasks in the monorepo
func (c *GraphCommand) Run(args []string) int {
	pref := color.New(color.Bold, color.FgRed, color.ReverseVideo).Sprint(" ERROR ")
	c.Ui.Output(fmt.Sprintf("%s%s", pref, color.RedString(" This command has been deprecated. Please use `turbo run <task1> <task2> --graph` instead.")))
	return 1
}
