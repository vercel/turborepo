package info

import (
	"os"

	"github.com/vercel/turbo/cli/internal/config"

	"github.com/vercel/turbo/cli/internal/cmdutil"

	"github.com/spf13/cobra"
)

// BinCmd returns the Cobra bin command
func BinCmd(helper *cmdutil.Helper) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "bin",
		Short: "Get the path to the Turbo binary",
		RunE: func(cmd *cobra.Command, args []string) error {
			base, err := helper.GetCmdBase(config.FlagSet{FlagSet: cmd.Flags()})
			if err != nil {
				return err
			}
			path, err := os.Executable()
			if err != nil {
				base.LogError("could not get path to turbo binary: %w", err)
				return err
			}

			base.UI.Output(path)

			return nil
		},
	}

	return cmd
}
