package auth

import (
	"os"

	"github.com/vercel/turbo/cli/internal/cmdutil"
	"github.com/vercel/turbo/cli/internal/turbostate"
	"github.com/vercel/turbo/cli/internal/util"
)

// RunLogout executes the `logout` command directly instead of via cobra.
func RunLogout(helper *cmdutil.Helper, args *turbostate.Args) error {
	base, err := helper.GetCmdBase(args)
	if err != nil {
		return err
	}
	if err := base.UserConfig.Delete(); err != nil && !os.IsNotExist(err) {
		base.LogError("could not logout. Something went wrong: %w", err)
		return err
	}

	base.UI.Info(util.Sprintf("${GREY}>>> Logged out${RESET}"))

	return nil
}
