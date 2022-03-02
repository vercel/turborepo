package auth

import (
	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/cmdutil"
	"github.com/vercel/turborepo/cli/internal/config"
)

func LogoutCmd(ch *cmdutil.Helper) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "logout",
		Short: "Logout of your Vercel account",
		RunE: func(cmd *cobra.Command, args []string) error {
			if err := config.DeleteUserConfigFile(); err != nil {
				return ch.LogError("could not logout. Something went wrong: %w", err)
			}

			ch.Logger.Printf("${GREY}>>> Logged out${RESET}")
			return nil
		},
	}

	return cmd
}
