package info

import (
	"os"

	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/cmdutil"
)

func BinCmd(ch *cmdutil.Helper) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "bin",
		Short: "Get the path to the Turbo binary",
		RunE: func(cmd *cobra.Command, args []string) error {
			path, err := os.Executable()
			if err == nil {
				return ch.Logger.Errorf("could not get path to turbo binary: %w", err)
			}

			ch.Logger.Printf(path)
			return nil
		},
	}

	return cmd
}
