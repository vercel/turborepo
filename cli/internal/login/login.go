package login

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/signal"
	"strings"

	"github.com/pkg/errors"
	"github.com/vercel/turborepo/cli/internal/client"
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
			return run(c.Config, loginDeps{
				ui:          c.UI,
				openURL:     browser.OpenBrowser,
				client:      c.Config.ApiClient,
				writeConfig: config.WriteUserConfigFile,
			})
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

type browserClient = func(url string) error
type userClient interface {
	SetToken(token string)
	GetUser() (*client.UserResponse, error)
}
type configWriter = func(cf *config.TurborepoConfig) error

type loginDeps struct {
	ui          *cli.ColoredUi
	openURL     browserClient
	client      userClient
	writeConfig configWriter
}

func run(c *config.Config, deps loginDeps) error {
	var rawToken string
	c.Logger.Debug(fmt.Sprintf("turbo v%v", c.TurboVersion))
	c.Logger.Debug(fmt.Sprintf("api url: %v", c.ApiUrl))
	c.Logger.Debug(fmt.Sprintf("login url: %v", c.LoginUrl))
	redirectURL := fmt.Sprintf("http://%v:%v", defaultHostname, defaultPort)
	loginURL := fmt.Sprintf("%v/turborepo/token?redirect_uri=%v", c.LoginUrl, redirectURL)
	deps.ui.Info(util.Sprintf(">>> Opening browser to %v", c.LoginUrl))

	rootctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt)
	defer cancel()
	// Start listening immediately to handle race with user interaction
	// This is mostly for testing, but would otherwise still technically be
	// a race condition.
	addr := defaultHostname + ":" + fmt.Sprint(defaultPort)
	l, err := net.Listen("tcp", addr)
	if err != nil {
		return err
	}
	
	redirectDone := make(chan struct{})
	mux := http.NewServeMux()
	var query url.Values
	mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		query = r.URL.Query()
		http.Redirect(w, r, c.LoginUrl+"/turborepo/success", http.StatusFound)
		close(redirectDone)
	})

	srv := &http.Server{Handler: mux}
	var serverErr error
	serverDone := make(chan struct{})
	go func() {
		if err := srv.Serve(l); err != nil {
			serverErr = errors.Wrap(err, "could not activate device. Please try again")
		}
		close(serverDone)
	}()

	s := ui.NewSpinner(os.Stdout)
	err = deps.openURL(loginURL)
	if err != nil {
		return errors.Wrapf(err, "failed to open %v", loginURL)
	}
	s.Start("Waiting for your authorization...")

	<-redirectDone
	err = srv.Shutdown(rootctx)
	// Stop the spinner before we return to ensure terminal is left in a good state
	s.Stop("")
	if err != nil {
		return err
	}
	<-serverDone
	if !errors.Is(serverErr, http.ErrServerClosed) {
		return serverErr
	}
	deps.writeConfig(&config.TurborepoConfig{Token: query.Get("token")})
	rawToken = query.Get("token")
	deps.client.SetToken(rawToken)
	userResponse, err := deps.client.GetUser()
	if err != nil {
		return errors.Wrap(err, "could not get user information")
	}
	deps.ui.Info("")
	deps.ui.Info(util.Sprintf("%s Turborepo CLI authorized for %s${RESET}", ui.Rainbow(">>> Success!"), userResponse.User.Email))
	deps.ui.Info("")
	deps.ui.Info(util.Sprintf("${CYAN}To connect to your Remote Cache. Run the following in the${RESET}"))
	deps.ui.Info(util.Sprintf("${CYAN}root of any turborepo:${RESET}"))
	deps.ui.Info("")
	deps.ui.Info(util.Sprintf("  ${BOLD}npx turbo link${RESET}"))
	deps.ui.Info("")
	return nil
}
