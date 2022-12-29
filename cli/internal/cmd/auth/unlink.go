package auth

import (
	"github.com/vercel/turbo/cli/internal/cmdutil"
	"github.com/vercel/turbo/cli/internal/turbostate"
	"github.com/vercel/turbo/cli/internal/util"
)

// ExecuteUnlink executes the `unlink` command directly instead of via cobra.
func ExecuteUnlink(helper *cmdutil.Helper, args *turbostate.ParsedArgsFromRust) error {
	base, err := helper.GetCmdBase(args)
	if err != nil {
		return err
	}

	if args.TestRun {
		base.UI.Info("Unlink test run successful")
		return nil
	}

	if err := base.RepoConfig.Delete(); err != nil {
		base.LogError("could not unlink. Something went wrong: %w", err)
		return err
	}

	base.UI.Output(util.Sprintf("${GREY}> Disabled Remote Caching${RESET}"))

	return nil
}
