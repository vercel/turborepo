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

// LoginCommand is a Command implementation allows the user to login to turbo
type LoginCommand struct {
	Config *config.Config
	Ui     *cli.ColoredUi
}

// Synopsis of run command
func (c *LoginCommand) Synopsis() string {
	return "Login to your Turborepo account"
}

// Help returns information about the `run` command
func (c *LoginCommand) Help() string {
	helpText := `
Usage: turbo login

    Login to your Turborepo account
`
	return strings.TrimSpace(helpText)
}

// Run executes tasks in the monorepo
func (c *LoginCommand) Run(args []string) int {
	var rawToken string

	c.Ui.Info(ui.Dim("Turborepo CLI"))
	c.Ui.Info(ui.Dim(c.Config.ApiUrl))

	if rawToken == "" {

		token, deviceTokenErr := c.Config.ApiClient.RequestDeviceToken()
		if deviceTokenErr != nil {
			c.logError(c.Config.Logger, "", fmt.Errorf("Could not request a device token. Please check your Internet connection.\n\n%s", ui.Dim(deviceTokenErr.Error())))
			return 1
		}

		c.Ui.Info(sprintf("To activate this machine, please visit ${BOLD}%s${RESET} and enter the code below:", token.VerificationUri))
		c.Ui.Info("")
		c.Ui.Info(sprintf("  Code: ${BOLD}%s${RESET}", token.UserCode))
		c.Ui.Info("")
		s := ui.NewSpinner(ui.Dim("Waiting for activation..."))
		s.Start()
		accessToken, accessTokenErr := c.Config.ApiClient.PollForAccessToken(token)
		if accessTokenErr != nil {
			c.logError(c.Config.Logger, "", fmt.Errorf("Could not activate device. Please try again: %w", accessTokenErr))
			return 1
		}
		s.Stop()

		config.WriteUserConfigFile(&config.TurborepoConfig{Token: accessToken.AccessToken})
		rawToken = accessToken.AccessToken
	}

	req, err := graphql.NewGetViewerRequest(c.Config.ApiUrl)
	req.Header.Set("Authorization", "Bearer "+rawToken)
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("Could not activate device. Please try again: %w", err))
		return 1
	}
	res, resErr := req.Execute(c.Config.GraphQLClient.Client)
	if resErr != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("Could not get user. Please try logging in again: %w", resErr))
		return 1
	}
	c.Ui.Info("")
	c.Ui.Info(sprintf("${GREEN}✓${RESET} Device activated for %s", res.Viewer.Email))
	return 0
}

// logError logs an error and outputs it to the UI.
func (c *LoginCommand) logError(log hclog.Logger, prefix string, err error) {
	log.Error(prefix, "error", err)

	if prefix != "" {
		prefix += ": "
	}

	c.Ui.Error(fmt.Sprintf("%s%s%s", ui.ERROR_PREFIX, prefix, color.RedString(" %v", err)))
}

// logError logs an error and outputs it to the UI.
func (c *LoginCommand) logWarning(log hclog.Logger, prefix string, err error) {
	log.Warn(prefix, "warning", err)

	if prefix != "" {
		prefix = " " + prefix + ": "
	}

	c.Ui.Error(fmt.Sprintf("%s%s%s", ui.WARNING_PREFIX, prefix, color.YellowString(" %v", err)))
}

// logError logs an error and outputs it to the UI.
func (c *LoginCommand) logFatal(log hclog.Logger, prefix string, err error) {
	log.Error(prefix, "error", err)

	if prefix != "" {
		prefix += ": "
	}

	c.Ui.Error(fmt.Sprintf("%s%s%s", ui.ERROR_PREFIX, prefix, color.RedString(" %v", err)))
	os.Exit(1)
}
