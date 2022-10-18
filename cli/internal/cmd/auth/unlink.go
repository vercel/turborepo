package auth

import (
	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/cmdutil"
	"github.com/vercel/turborepo/cli/internal/turbostate"
	"github.com/vercel/turborepo/cli/internal/util"
)

// UnlinkCmd returns the Cobra unlink command
func UnlinkCmd(helper *cmdutil.Helper) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "unlink",
		Short: "Unlink the current directory from your Vercel organization and disable Remote Caching",
		RunE: func(cmd *cobra.Command, args []string) error {
			base, err := helper.GetCmdBase(cmd.Flags())
			if err != nil {
				return err
			}
			if err := base.RepoConfig.Delete(); err != nil {
				base.LogError("could not unlink. Something went wrong: %w", err)
				return err
			}

			base.UI.Output(util.Sprintf("${GREY}> Disabled Remote Caching${RESET}"))

			return nil
		},
	}

	return cmd
}

func RunUnlink(helper *cmdutil.Helper, args *turbostate.Args) error {
	base, err := helper.GetCmdBaseFromArgs(args)
	if err != nil {
		return err
	}
	if err := base.RepoConfig.Delete(); err != nil {
		base.LogError("could not unlink. Something went wrong: %w", err)
		return err
	}

	base.UI.Output(util.Sprintf("${GREY}> Disabled Remote Caching${RESET}"))

	return nil
}
