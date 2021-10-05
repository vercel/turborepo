package login

import (
	"fmt"
	"os"
	"strings"
	"turbo/internal/config"
	"turbo/internal/graphql"
	"turbo/internal/ui"

	"github.com/fatih/color"
	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
)

// MeCommand is a Command implementation allows the user to login to turbo
type MeCommand struct {
	Config *config.Config
	Ui     *cli.ColoredUi
}

// Synopsis of run command
func (c *MeCommand) Synopsis() string {
	return "Print information about your Turborepo.com Account"
}

// Help returns information about the `run` command
func (c *MeCommand) Help() string {
	helpText := `
Usage: turbo me

Print information about your Turborepo.com Account
`
	return strings.TrimSpace(helpText)
}

// Run executes tasks in the monorepo
func (c *MeCommand) Run(args []string) int {
	req, err := graphql.NewGetViewerRequest(c.Config.ApiUrl)
	req.Header.Set("Authorization", "Bearer "+c.Config.Token)
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("Could not activate device. Please try again: %w", err))
		return 0
	}

	res, resErr := req.Execute(c.Config.GraphQLClient.Client)
	if resErr != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("Could not get user. Please try logging in again: %w", resErr))
		return 0
	}

	c.Ui.Info("")
	c.Ui.Info(fmt.Sprintf("user %v", res.Viewer.Email))
	c.Ui.Info("")
	return 1
}

// logError logs an error and outputs it to the UI.
func (c *MeCommand) logError(log hclog.Logger, prefix string, err error) {
	log.Error(prefix, "error", err)

	if prefix != "" {
		prefix += ": "
	}

	c.Ui.Error(fmt.Sprintf("%s%s%s", ui.ERROR_PREFIX, prefix, color.RedString(" %v", err)))
}

// logError logs an error and outputs it to the UI.
func (c *MeCommand) logWarning(log hclog.Logger, prefix string, err error) {
	log.Warn(prefix, "warning", err)

	if prefix != "" {
		prefix = " " + prefix + ": "
	}

	c.Ui.Error(fmt.Sprintf("%s%s%s", ui.WARNING_PREFIX, prefix, color.YellowString(" %v", err)))
}

// logError logs an error and outputs it to the UI.
func (c *MeCommand) logFatal(log hclog.Logger, prefix string, err error) {
	log.Error(prefix, "error", err)

	if prefix != "" {
		prefix += ": "
	}

	c.Ui.Error(fmt.Sprintf("%s%s%s", ui.ERROR_PREFIX, prefix, color.RedString(" %v", err)))
	os.Exit(1)
}
