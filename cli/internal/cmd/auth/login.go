package auth

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/signal"

	"github.com/pkg/errors"
	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/cmdutil"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util/browser"
)

const (
	defaultHostname    = "127.0.0.1"
	defaultPort        = 9789
	defaultSSOProvider = "SAML/OIDC Single Sign-On"
)

type oneShotServer struct {
	Port        uint16
	requestDone chan struct{}
	serverDone  chan struct{}
	serverErr   error
	ctx         context.Context
	srv         *http.Server
}

type deps struct {
	ch      *cmdutil.Helper
	openUrl func(string) error
}

func LoginCmd(ch *cmdutil.Helper) *cobra.Command {
	var opts struct {
		ssoTeam string
	}

	cmd := &cobra.Command{
		Use:   "login",
		Short: "Login to your Vercel account",
		RunE: func(cmd *cobra.Command, args []string) error {
			if opts.ssoTeam != "" {
				return loginSSO(deps{
					ch:      ch,
					openUrl: browser.OpenBrowser,
				}, opts.ssoTeam)
			} else {
				return login(deps{
					ch:      ch,
					openUrl: browser.OpenBrowser,
				})
			}
		},
	}

	cmd.Flags().StringVar(&opts.ssoTeam, "sso-team", "", "attempt to authenticate to the specified team using SSO")

	return cmd
}

func login(deps deps) error {
	ch := deps.ch

	ch.Config.Logger.Debug(fmt.Sprintf("turbo v%v", ch.Config.Version))
	ch.Config.Logger.Debug(fmt.Sprintf("api url: %v", ch.Config.ApiUrl))
	ch.Config.Logger.Debug(fmt.Sprintf("login url: %v", ch.Config.LoginUrl))
	redirectURL := fmt.Sprintf("http://%v:%v", defaultHostname, defaultPort)
	loginURL := fmt.Sprintf("%v/turborepo/token?redirect_uri=%v", ch.Config.LoginUrl, redirectURL)
	ch.Logger.Printf(">>> Opening browser to %v", ch.Config.LoginUrl)

	rootctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt)
	defer cancel()

	var query url.Values
	oss, err := newOneShotServer(rootctx, func(w http.ResponseWriter, r *http.Request) {
		query = r.URL.Query()
		http.Redirect(w, r, ch.Config.LoginUrl+"/turborepo/success", http.StatusFound)
	}, defaultPort)
	if err != nil {
		return ch.LogError("failed to start local server: %w", err)
	}

	s := ui.NewSpinner(os.Stdout)
	err = deps.openUrl(loginURL)
	if err != nil {
		return ch.LogError("failed to open %v: %w", loginURL, err)
	}
	s.Start("Waiting for your authorization...")
	err = oss.Wait()
	if err != nil {
		return ch.LogError("failed to shut down local server: %w", err)
	}
	// Stop the spinner before we return to ensure terminal is left in a good state
	s.Stop("")

	rawToken := query.Get("token")
	config.WriteUserConfigFile(&config.TurborepoConfig{Token: rawToken})
	ch.Config.ApiClient.SetToken(rawToken)

	userResponse, err := ch.Config.ApiClient.GetUser()
	if err != nil {
		return ch.LogError("could not get user information: %w", err)
	}

	ch.Logger.Printf("")
	ch.Logger.Printf("%s Turborepo CLI authorized for %s${RESET}", ui.Rainbow(">>> Success!"), userResponse.User.Email)
	ch.Logger.Printf("")
	ch.Logger.Printf("${CYAN}To connect to your Remote Cache. Run the following in the${RESET}")
	ch.Logger.Printf("${CYAN}root of any turborepo:${RESET}")
	ch.Logger.Printf("")
	ch.Logger.Printf("  ${BOLD}npx turbo link${RESET}")
	ch.Logger.Printf("")

	return nil
}

func loginSSO(deps deps, ssoTeam string) error {
	ch := deps.ch

	redirectURL := fmt.Sprintf("http://%v:%v", defaultHostname, defaultPort)
	query := make(url.Values)
	query.Add("teamId", ssoTeam)
	query.Add("mode", "login")
	query.Add("next", redirectURL)
	loginURL := fmt.Sprintf("%v/api/auth/sso?%v", ch.Config.LoginUrl, query.Encode())

	rootctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt)
	defer cancel()

	var verificationToken string
	oss, err := newOneShotServer(rootctx, func(w http.ResponseWriter, r *http.Request) {
		token, location := getTokenAndRedirect(r.URL.Query())
		verificationToken = token
		http.Redirect(w, r, location, http.StatusFound)
	}, defaultPort)
	if err != nil {
		return ch.LogError("failed to start local server: %v", err)
	}

	s := ui.NewSpinner(os.Stdout)
	err = deps.openUrl(loginURL)
	if err != nil {
		return ch.LogError("failed to open %v: %w", loginURL, err)
	}

	s.Start("Waiting for your authorization...")
	err = oss.Wait()
	if err != nil {
		return ch.LogError("failed to shut down local server: %w", err)
	}

	// Stop the spinner before we return to ensure terminal is left in a good state
	s.Stop("")
	// open https://vercel.com/api/auth/sso?teamId=<TEAM_ID>&mode=login
	if verificationToken == "" {
		return ch.LogError("no token auth token found")
	}

	// We now have a verification token. We need to pass it to the verification endpoint
	// to get an actual token.
	tokenName, err := makeTokenName()
	if err != nil {
		return ch.LogError("failed to make sso token name: %w", err)
	}
	verifiedUser, err := ch.Config.ApiClient.VerifySSOToken(verificationToken, tokenName)
	if err != nil {
		return ch.LogError("failed to verify SSO token: %w", err)
	}

	ch.Config.ApiClient.SetToken(verifiedUser.Token)
	userResponse, err := ch.Config.ApiClient.GetUser()
	if err != nil {
		return ch.LogError("could not get user information: %w", err)
	}

	err = config.WriteUserConfigFile(&config.TurborepoConfig{Token: verifiedUser.Token})
	if err != nil {
		return ch.LogError("failed to save auth token: %w", err)
	}

	ch.Logger.Printf("")
	ch.Logger.Printf("%s Turborepo CLI authorized for %s${RESET}", ui.Rainbow(">>> Success!"), userResponse.User.Email)
	ch.Logger.Printf("")

	if verifiedUser.TeamID != "" {
		err = config.WriteRepoConfigFile(&config.TurborepoConfig{TeamId: verifiedUser.TeamID, ApiUrl: ch.Config.ApiUrl})
		if err != nil {
			return ch.LogError("failed to save teamId: %w", err)
		}
	} else {
		ch.Logger.Printf("${CYAN}To connect to your Remote Cache. Run the following in the${RESET}")
		ch.Logger.Printf("${CYAN}root of any turborepo:${RESET}")
		ch.Logger.Printf("")
		ch.Logger.Printf("  ${BOLD}npx turbo link${RESET}")
		ch.Logger.Printf("")
	}

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
			oss.serverErr = fmt.Errorf("could not activate device. Please try again: %w", err)
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
