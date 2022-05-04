package info

import (
	"errors"
	"fmt"
	"os"

	"github.com/fatih/color"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"

	"github.com/mitchellh/cli"
	"github.com/spf13/cobra"
)

// BinCommand is the structure for the bin command
type BinCommand struct {
	Config *config.Config
	UI     *cli.ColoredUi
}

// Synopsis of the bin command
func (c *BinCommand) Synopsis() string {
	return BinCmd(c).Short
}

// Help returns information about the bin command
func (c *BinCommand) Help() string {
	return util.HelpForCobraCmd(BinCmd(c))
}

// Run setups the command and runs it
func (c *BinCommand) Run(args []string) int {
	cmd := BinCmd(c)

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

// LogError prints an error to the UI and returns a BasicError
func (c *BinCommand) LogError(format string, args ...interface{}) error {
	err := fmt.Errorf(format, args...)
	c.Config.Logger.Error("error", err)
	c.UI.Error(fmt.Sprintf("%s%s", ui.ERROR_PREFIX, color.RedString(" %v", err)))
	return err
}

// BinCmd returns the Cobra bin command
func BinCmd(ch *BinCommand) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "bin",
		Short: "Get the path to the Turbo binary",
		RunE: func(cmd *cobra.Command, args []string) error {
			path, err := os.Executable()
			if err != nil {
				return ch.LogError("could not get path to turbo binary: %w", err)
			}

			ch.UI.Output(path)

			return nil
		},
	}

	return cmd
}
