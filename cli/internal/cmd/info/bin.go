package info

import (
	"errors"
	"os"
	"strings"

	"github.com/vercel/turborepo/cli/internal/cmdutil"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/logger"

	"github.com/mitchellh/cli"
	"github.com/spf13/cobra"
)

type BinCommand struct {
	Config *config.Config
	Ui     *cli.ColoredUi
}

// Synopsis of run command
func (c *BinCommand) Synopsis() string {
	return "Get the path to the Turbo binary"
}

// Help returns information about the `bin` command
func (c *BinCommand) Help() string {
	helpText := `
Usage: turbo bin

  Get the path to the Turbo binary
`
	return strings.TrimSpace(helpText)
}

func (c *BinCommand) Run(args []string) int {
	logger := logger.New()
	cmd := BinCmd(&cmdutil.Helper{
		Config: c.Config,
		Logger: logger,
	})

	cmd.SilenceErrors = true
	cmd.CompletionOptions.DisableDefaultCmd = true

	cmd.SetArgs(args)

	err := cmd.Execute()
	if err == nil {
		return 0
	}

	logger.Error(err)

	var cmdErr *cmdutil.Error
	if errors.As(err, &cmdErr) {
		return cmdErr.ExitCode
	}

	return 1
}

func BinCmd(ch *cmdutil.Helper) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "bin",
		Short: "Get the path to the Turbo binary",
		RunE: func(cmd *cobra.Command, args []string) error {
			path, err := os.Executable()
			if err != nil {
				return ch.LogError("could not get path to turbo binary: %w", err)
			}

			ch.Logger.Printf(path)
			return nil
		},
	}

	return cmd
}
