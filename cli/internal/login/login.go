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

// Help returns information about the `run` command. Match the cobra output for now, until
// we can wire up cobra for real
func (c *LoginCommand) Help() string {
	helpText := `
Login to your Vercel account

Usage:
  turbo login [flags]

Flags:
      --sso-team string   attempt to authenticate to the specified team using SSO
`
	return strings.TrimSpace(helpText)
}

const defaultHostname = "127.0.0.1"
const defaultPort = 9789
const defaultSSOProvider = "SAML/OIDC Single Sign-On"

// Run logs into the api with PKCE and writes the token to turbo user config directory
func (c *LoginCommand) Run(args []string) int {
	var ssoTeam string
	loginCommand := &cobra.Command{
		Use:   "turbo login",
		Short: "Login to your Vercel account",
		RunE: func(cmd *cobra.Command, args []string) error {
			deps := loginDeps{
				ui:              c.UI,
				openURL:         browser.OpenBrowser,
				client:          c.Config.ApiClient,
				writeUserConfig: config.WriteUserConfigFile,
				writeRepoConfig: config.WriteRepoConfigFile,
			}
			if ssoTeam != "" {
				return loginSSO(c.Config, ssoTeam, deps)
			}
			return run(c.Config, deps)
		},
	}
	loginCommand.Flags().StringVar(&ssoTeam, "sso-team", "", "attempt to authenticate to the specified team using SSO")
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
	VerifySSOToken(token string, tokenName string) (*client.VerifiedSSOUser, error)
}
type configWriter = func(cf *config.TurborepoConfig) error

type loginDeps struct {
	ui              *cli.ColoredUi
	openURL         browserClient
	client          userClient
	writeUserConfig configWriter
	writeRepoConfig configWriter
}

func run(c *config.Config, deps loginDeps) error {
	c.Logger.Debug(fmt.Sprintf("turbo v%v", c.TurboVersion))
	c.Logger.Debug(fmt.Sprintf("api url: %v", c.ApiUrl))
	c.Logger.Debug(fmt.Sprintf("login url: %v", c.LoginUrl))
	redirectURL := fmt.Sprintf("http://%v:%v", defaultHostname, defaultPort)
	loginURL := fmt.Sprintf("%v/turborepo/token?redirect_uri=%v", c.LoginUrl, redirectURL)
	deps.ui.Info(util.Sprintf(">>> Opening browser to %v", c.LoginUrl))

	rootctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt)
	defer cancel()

	var query url.Values
	oss, err := newOneShotServer(rootctx, func(w http.ResponseWriter, r *http.Request) {
		query = r.URL.Query()
		http.Redirect(w, r, c.LoginUrl+"/turborepo/success", http.StatusFound)
	}, defaultPort)
	if err != nil {
		return errors.Wrap(err, "failed to start local server")
	}

	s := ui.NewSpinner(os.Stdout)
	err = deps.openURL(loginURL)
	if err != nil {
		return errors.Wrapf(err, "failed to open %v", loginURL)
	}
	s.Start("Waiting for your authorization...")
	err = oss.Wait()
	if err != nil {
		return errors.Wrap(err, "failed to shut down local server")
	}
	// Stop the spinner before we return to ensure terminal is left in a good state
	s.Stop("")

	deps.writeUserConfig(&config.TurborepoConfig{Token: query.Get("token")})
	rawToken := query.Get("token")
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

func loginSSO(c *config.Config, ssoTeam string, deps loginDeps) error {
	redirectURL := fmt.Sprintf("http://%v:%v", defaultHostname, defaultPort)
	query := make(url.Values)
	query.Add("teamId", ssoTeam)
	query.Add("mode", "login")
	query.Add("next", redirectURL)
	loginURL := fmt.Sprintf("%v/api/auth/sso?%v", c.LoginUrl, query.Encode())

	rootctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt)
	defer cancel()

	var verificationToken string
	oss, err := newOneShotServer(rootctx, func(w http.ResponseWriter, r *http.Request) {
		token, location := getTokenAndRedirect(r.URL.Query())
		verificationToken = token
		http.Redirect(w, r, location, http.StatusFound)
	}, defaultPort)
	if err != nil {
		return errors.Wrap(err, "failed to start local server")
	}
	s := ui.NewSpinner(os.Stdout)
	err = deps.openURL(loginURL)
	if err != nil {
		return errors.Wrapf(err, "failed to open %v", loginURL)
	}
	s.Start("Waiting for your authorization...")
	err = oss.Wait()
	if err != nil {
		return errors.Wrap(err, "failed to shut down local server")
	}
	// Stop the spinner before we return to ensure terminal is left in a good state
	s.Stop("")
	// open https://vercel.com/api/auth/sso?teamId=<TEAM_ID>&mode=login
	if verificationToken == "" {
		return errors.New("no token auth token found")
	}

	// We now have a verification token. We need to pass it to the verification endpoint
	// to get an actual token.
	tokenName, err := makeTokenName()
	if err != nil {
		return errors.Wrap(err, "failed to make sso token name")
	}
	verifiedUser, err := deps.client.VerifySSOToken(verificationToken, tokenName)
	if err != nil {
		return errors.Wrap(err, "failed to verify SSO token")
	}

	deps.client.SetToken(verifiedUser.Token)
	userResponse, err := deps.client.GetUser()
	if err != nil {
		return errors.Wrap(err, "could not get user information")
	}
	err = deps.writeUserConfig(&config.TurborepoConfig{Token: verifiedUser.Token})
	if err != nil {
		return errors.Wrap(err, "failed to save auth token")
	}
	deps.ui.Info("")
	deps.ui.Info(util.Sprintf("%s Turborepo CLI authorized for %s${RESET}", ui.Rainbow(">>> Success!"), userResponse.User.Email))
	deps.ui.Info("")
	if verifiedUser.TeamID != "" {
		err = deps.writeRepoConfig(&config.TurborepoConfig{TeamId: verifiedUser.TeamID, ApiUrl: c.ApiUrl})
		if err != nil {
			return errors.Wrap(err, "failed to save teamId")
		}
	} else {

		deps.ui.Info(util.Sprintf("${CYAN}To connect to your Remote Cache. Run the following in the${RESET}"))
		deps.ui.Info(util.Sprintf("${CYAN}root of any turborepo:${RESET}"))
		deps.ui.Info("")
		deps.ui.Info(util.Sprintf("  ${BOLD}npx turbo link${RESET}"))
	}
	deps.ui.Info("")
	return nil
}

func getTokenAndRedirect(params url.Values) (string, string) {
	locationStub := "https://vercel.com/notifications/cli-login-"
	if loginError := params.Get("loginError"); loginError != "" {
		outParams := make(url.Values)
		outParams.Add("loginError", loginError)
		return "", locationStub + "failed?" + outParams.Encode()
	}
	if ssoEmail := params.Get("ssoEmail"); ssoEmail != "" {
		outParams := make(url.Values)
		outParams.Add("ssoEmail", ssoEmail)
		if teamName := params.Get("teamName"); teamName != "" {
			outParams.Add("teamName", teamName)
		}
		if ssoType := params.Get("ssoType"); ssoType != "" {
			outParams.Add("ssoType", ssoType)
		}
		return "", locationStub + "incomplete?" + outParams.Encode()
	}
	token := params.Get("token")
	location := locationStub + "success"
	if email := params.Get("email"); email != "" {
		outParams := make(url.Values)
		outParams.Add("email", email)
		location += "?" + outParams.Encode()
	}
	return token, location
}

type oneShotServer struct {
	Port        uint16
	requestDone chan struct{}
	serverDone  chan struct{}
	serverErr   error
	ctx         context.Context
	srv         *http.Server
}

func newOneShotServer(ctx context.Context, handler http.HandlerFunc, port uint16) (*oneShotServer, error) {
	requestDone := make(chan struct{})
	serverDone := make(chan struct{})
	mux := http.NewServeMux()
	srv := &http.Server{Handler: mux}
	oss := &oneShotServer{
		Port:        port,
		requestDone: requestDone,
		serverDone:  serverDone,
		ctx:         ctx,
		srv:         srv,
	}
	mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		handler(w, r)
		close(oss.requestDone)
	})
	err := oss.start(handler)
	if err != nil {
		return nil, err
	}
	return oss, nil
}

func (oss *oneShotServer) start(handler http.HandlerFunc) error {
	// Start listening immediately to handle race with user interaction
	// This is mostly for testing, but would otherwise still technically be
	// a race condition.
	addr := defaultHostname + ":" + fmt.Sprint(oss.Port)
	l, err := net.Listen("tcp", addr)
	if err != nil {
		return err
	}
	go func() {
		if err := oss.srv.Serve(l); err != nil && !errors.Is(err, http.ErrServerClosed) {
			oss.serverErr = errors.Wrap(err, "could not activate device. Please try again")
		}
		close(oss.serverDone)
	}()
	return nil
}

func (oss *oneShotServer) Wait() error {
	select {
	case <-oss.requestDone:
	case <-oss.ctx.Done():
	}
	return oss.closeServer()
}

func (oss *oneShotServer) closeServer() error {
	err := oss.srv.Shutdown(oss.ctx)
	if err != nil {
		return err
	}
	<-oss.serverDone
	return oss.serverErr
}

func makeTokenName() (string, error) {
	host, err := os.Hostname()
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("Turbo CLI on %v via %v", host, defaultSSOProvider), nil
}
