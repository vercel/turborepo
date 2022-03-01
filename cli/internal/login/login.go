package login

import (
	"context"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"os/signal"
	"strings"

	"github.com/pkg/errors"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"
	"github.com/vercel/turborepo/cli/internal/util/browser"

	"github.com/fatih/color"
	"github.com/mitchellh/cli"
	"github.com/spf13/cobra"
)

// LoginCommand is a Command implementation allows the user to login to turbo
type LoginCommand struct {
	Config *config.Config
	UI     *cli.ColoredUi
}

// Synopsis of run command
func (c *LoginCommand) Synopsis() string {
	return "Login to your Vercel account"
}

// Help returns information about the `run` command
func (c *LoginCommand) Help() string {
	helpText := `
Usage: turbo login

    Login to your Vercel account
`
	return strings.TrimSpace(helpText)
}

const defaultHostname = "127.0.0.1"
const defaultPort = 9789

// Run logs into the api with PKCE and writes the token to turbo user config directory
func (c *LoginCommand) Run(args []string) int {
	loginCommand := &cobra.Command{
		Use:   "turbo login",
		Short: "Login to your Vercel account",
		RunE: func(cmd *cobra.Command, args []string) error {
			return run(c.Config, c.UI)
		},
	}
	loginCommand.SetArgs(args)
	err := loginCommand.Execute()
	if err != nil {
		c.Config.Logger.Error("error", err)
		c.UI.Error(fmt.Sprintf("%s%s", ui.ERROR_PREFIX, color.RedString(" %v", err)))
		return 1
	}
	return 0
}

func run(c *config.Config, tui *cli.ColoredUi) error {
	var rawToken string
	c.Logger.Debug(fmt.Sprintf("turbo v%v", c.TurboVersion))
	c.Logger.Debug(fmt.Sprintf("api url: %v", c.ApiUrl))
	c.Logger.Debug(fmt.Sprintf("login url: %v", c.LoginUrl))
	redirectURL := fmt.Sprintf("http://%v:%v", defaultHostname, defaultPort)
	loginURL := fmt.Sprintf("%v/turborepo/token?redirect_uri=%v", c.LoginUrl, redirectURL)
	tui.Info(util.Sprintf(">>> Opening browser to %v", c.LoginUrl))
	s := ui.NewSpinner(os.Stdout)
	browser.OpenBrowser(loginURL)
	s.Start("Waiting for your authorization...")

	var query url.Values
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt)
	fmt.Println(query.Encode())
	http.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		query = r.URL.Query()
		http.Redirect(w, r, c.LoginUrl+"/turborepo/success", http.StatusFound)
		cancel()
	})

	srv := &http.Server{Addr: defaultHostname + ":" + fmt.Sprint(defaultPort)}
	var serverErr error
	go func() {
		if err := srv.ListenAndServe(); err != nil {
			if err != nil {
				serverErr = errors.Wrap(err, "could not activate device. Please try again")
			}
		}
	}()
	<-ctx.Done()
	s.Stop("")
	if serverErr != nil {
		return serverErr
	}
	err := srv.Close()
	if err != nil {
		return err
	}
	config.WriteUserConfigFile(&config.TurborepoConfig{Token: query.Get("token")})
	rawToken = query.Get("token")
	c.ApiClient.SetToken(rawToken)
	userResponse, err := c.ApiClient.GetUser()
	if err != nil {
		return errors.Wrap(err, "could not get user information")
	}
	tui.Info("")
	tui.Info(util.Sprintf("%s Turborepo CLI authorized for %s${RESET}", ui.Rainbow(">>> Success!"), userResponse.User.Email))
	tui.Info("")
	tui.Info(util.Sprintf("${CYAN}To connect to your Remote Cache. Run the following in the${RESET}"))
	tui.Info(util.Sprintf("${CYAN}root of any turborepo:${RESET}"))
	tui.Info("")
	tui.Info(util.Sprintf("  ${BOLD}npx turbo link${RESET}"))
	tui.Info("")
	return nil
}
