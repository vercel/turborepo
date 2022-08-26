package auth

import (
	"errors"
	"fmt"

	"github.com/fatih/color"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"

	"github.com/mitchellh/cli"
	"github.com/spf13/cobra"
)

// LogoutCommand is the structure for the logout command
type LogoutCommand struct {
	Config *config.Config
	UI     *cli.ColoredUi
}

// Synopsis of the logout command
func (c *LogoutCommand) Synopsis() string {
	return LogoutCmd(c).Short
}

// Help returns information about the logout command
func (c *LogoutCommand) Help() string {
	return util.HelpForCobraCmd(LogoutCmd(c))
}

// Run setups the command and runs it
func (c *LogoutCommand) Run(args []string) int {
	cmd := LogoutCmd(c)

	cmd.SilenceUsage = true
	cmd.SilenceErrors = true
	cmd.CompletionOptions.DisableDefaultCmd = true

	cmd.SetArgs(args)

	err := cmd.Execute()
	if err == nil {
		return 0
	}

	var cmdErr *util.ExitCodeError
	if errors.As(err, &cmdErr) {
		return cmdErr.ExitCode
	}

	return 1
}

// logError prints an error to the UI and returns a formatted error
func (c *LogoutCommand) logError(format string, args ...interface{}) error {
	err := fmt.Errorf(format, args...)
	c.Config.Logger.Error("error", err)
	c.UI.Error(fmt.Sprintf("%s%s", ui.ERROR_PREFIX, color.RedString(" %v", err)))
	return err
}

// LogoutCmd returns the Cobra logout command
func LogoutCmd(ch *LogoutCommand) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "logout",
		Short: "Logout of your Vercel account",
		RunE: func(cmd *cobra.Command, args []string) error {
			if err := ch.Config.UserConfig.Delete(); err != nil {
				return ch.logError("could not logout. Something went wrong: %w", err)
			}

			ch.UI.Info(util.Sprintf("${GREY}>>> Logged out${RESET}"))

			return nil
		},
	}

	return cmd
}
