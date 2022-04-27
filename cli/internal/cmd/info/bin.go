package info

import (
	"errors"
	"fmt"
	"os"
	"strings"

	"github.com/vercel/turborepo/cli/internal/cmdutil"
	"github.com/vercel/turborepo/cli/internal/config"

	"github.com/mitchellh/cli"
	"github.com/spf13/cobra"
)

// BinCommand is the structure for the bin command
type BinCommand struct {
	Config *config.Config
	UI     *cli.ColoredUi
}

// Synopsis of run command
func (c *BinCommand) Synopsis() string {
	return "Get the path to the Turbo binary"
}

// Help returns information about the bin command
func (c *BinCommand) Help() string {
	helpText := `
Usage: turbo bin

  Get the path to the Turbo binary
`
	return strings.TrimSpace(helpText)
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

	var cmdErr *cmdutil.Error
	if errors.As(err, &cmdErr) {
		return cmdErr.ExitCode
	}

	return 1
}

// LogError prints an error to the UI and returns a BasicError
func (c *BinCommand) LogError(format string, args ...interface{}) error {
	err := fmt.Errorf(format, args...)
	c.Config.Logger.Error("error", err)
	c.UI.Error(err.Error())
	return &cmdutil.BasicError{}
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
