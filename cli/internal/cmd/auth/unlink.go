package auth

import (
	"github.com/spf13/cobra"
	"github.com/vercel/turbo/cli/internal/cmdutil"
	"github.com/vercel/turbo/cli/internal/turbostate"
	"github.com/vercel/turbo/cli/internal/util"
)

// RunUnlink executes the `unlink` command directly instead of via cobra.
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
