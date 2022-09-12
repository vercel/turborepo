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

// UnlinkCommand is the structure for the unlink command
type UnlinkCommand struct {
	Config *config.Config
	UI     *cli.ColoredUi
}

// Synopsis of the unlink command
func (c *UnlinkCommand) Synopsis() string {
	return UnlinkCmd(c).Short
}

// Help returns information about the unlink command
func (c *UnlinkCommand) Help() string {
	return util.HelpForCobraCmd(UnlinkCmd(c))
}

// Run setups the command and runs it
func (c *UnlinkCommand) Run(args []string) int {
	cmd := UnlinkCmd(c)

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
func (c *UnlinkCommand) logError(format string, args ...interface{}) error {
	err := fmt.Errorf(format, args...)
	c.Config.Logger.Error("error", err)
	c.UI.Error(fmt.Sprintf("%s%s", ui.ERROR_PREFIX, color.RedString(" %v", err)))
	return err
}

// UnlinkCmd returns the Cobra unlink command
func UnlinkCmd(ch *UnlinkCommand) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "unlink",
		Short: "Unlink the current directory from your Vercel organization and disable Remote Caching",
		RunE: func(cmd *cobra.Command, args []string) error {
			if err := ch.Config.RepoConfig.Delete(); err != nil {
				return ch.logError("could not unlink. Something went wrong: %w", err)
			}

			ch.UI.Output(util.Sprintf("${GREY}> Disabled Remote Caching${RESET}"))

			return nil
		},
	}

	return cmd
}
