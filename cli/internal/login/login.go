package login

import (
	"bytes"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"turbo/internal/config"
	"turbo/internal/ui"
	"turbo/internal/util"

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
	return "Login to your Vercel account"
}

// Help returns information about the `run` command
func (c *LoginCommand) Help() string {
	helpText := `
Usage: turbo login

    Login to your Vercel account
`
	return strings.TrimSpace(helpText)
}

// Run logs into the api with PKCE and writes the token to turbo user config directory
func (c *LoginCommand) Run(args []string) int {
	var rawToken string
	c.Config.Logger.Debug(fmt.Sprintf("turbo v%v", c.Config.TurboVersion))
	c.Config.Logger.Debug(fmt.Sprintf("api url: %v", c.Config.ApiUrl))
	c.Config.Logger.Debug(fmt.Sprintf("login url: %v", c.Config.LoginUrl))

	c.Ui.Info(util.Sprintf(">>> Opening browser to ${UNDERLINE}%v${RESET}", c.Config.LoginUrl))
	s := ui.NewSpinner(os.Stdout)
	s.Start("Waiting for your authorization...")
	c.Config.Logger.Debug(fmt.Sprintf("running `node %v`", filepath.FromSlash("./node_modules/turbo/login.js")))
	cmd := exec.Command("node", filepath.FromSlash("./node_modules/turbo/login.js"))
	var outb, errb bytes.Buffer
	cmd.Args = append(cmd.Args, c.Config.LoginUrl)
	cmd.Stdout = &outb
	cmd.Stderr = &errb

	err := cmd.Run()
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("Could not activate device. Please try again: %w", err))
		return 1
	}
	s.Stop("")
	config.WriteUserConfigFile(&config.TurborepoConfig{Token: outb.String()})
	rawToken = outb.String()

	if errb.String() != "" {
		c.logError(c.Config.Logger, "", fmt.Errorf("could not authorize: %s", errb.String()))
		return 1
	}

	c.Config.ApiClient.SetToken(rawToken)
	userResponse, err := c.Config.ApiClient.GetUser()
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("could not get user information.\n: %w", err))
		return 1
	}
	c.Ui.Info("")
	c.Ui.Info(util.Sprintf("%s Turborepo CLI authorized for %s${RESET}", ui.Rainbow(">>> Success!"), userResponse.User.Email))
	c.Ui.Info("")
	c.Ui.Info(util.Sprintf("${CYAN}To connect to your Remote Cache. Run the following in the${RESET}"))
	c.Ui.Info(util.Sprintf("${CYAN}root of any turborepo:${RESET}"))
	c.Ui.Info("")
	c.Ui.Info(util.Sprintf("  ${BOLD}npx turbo link${RESET}"))
	c.Ui.Info("")
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
