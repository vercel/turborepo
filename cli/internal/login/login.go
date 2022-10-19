package login

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/signal"

	"github.com/pkg/errors"
	"github.com/vercel/turborepo/cli/internal/client"
	"github.com/vercel/turborepo/cli/internal/cmdutil"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"
	"github.com/vercel/turborepo/cli/internal/util/browser"

	"github.com/spf13/cobra"
)

const defaultHostname = "127.0.0.1"
const defaultPort = 9789
const defaultSSOProvider = "SAML/OIDC Single Sign-On"

// NewLoginCommand returns the cobra subcommand for turbo login
func NewLoginCommand(helper *cmdutil.Helper) *cobra.Command {
	var ssoTeam string
	cmd := &cobra.Command{
		Use:           "login",
		Short:         "Login to your Vercel account",
		SilenceErrors: true,
		SilenceUsage:  true,
		RunE: func(cmd *cobra.Command, args []string) error {
			ctx := cmd.Context()
			base, err := helper.GetCmdBase(cmd.Flags())
			if err != nil {
				return err
			}
			login := login{
				base:                base,
				openURL:             browser.OpenBrowser,
				client:              base.APIClient,
				promptEnableCaching: promptEnableCaching,
			}
			if ssoTeam != "" {
				err := login.loginSSO(ctx, ssoTeam)
				if err != nil {
					if errors.Is(err, errUserCanceled) || errors.Is(err, context.Canceled) {
						base.UI.Info("Canceled. Turborepo not set up.")
					} else if errors.Is(err, errTryAfterEnable) || errors.Is(err, errNeedCachingEnabled) || errors.Is(err, errOverage) {
						base.UI.Info("Remote Caching not enabled. Please run 'turbo login' again after Remote Caching has been enabled")
					} else {
						base.LogError("SSO login failed: %v", err)
					}
					return err
				}
			} else {
				err := login.run(ctx)
				if err != nil {
					if errors.Is(err, context.Canceled) {
						base.UI.Info("Canceled. Turborepo not set up.")
					} else {
						base.LogError("login failed: %v", err)
					}
					return err
				}
			}
			return nil
		},
	}
	cmd.Flags().StringVar(&ssoTeam, "sso-team", "", "attempt to authenticate to the specified team using SSO")
	return cmd
}

type browserClient = func(url string) error
type userClient interface {
	SetToken(token string)
	GetUser() (*client.UserResponse, error)
	VerifySSOToken(token string, tokenName string) (*client.VerifiedSSOUser, error)
	SetTeamID(teamID string)
	GetCachingStatus() (util.CachingStatus, error)
	GetTeam(teamID string) (*client.Team, error)
}

type login struct {
	base    *cmdutil.CmdBase
	openURL browserClient
	// Use our own interface, rather than base.APIClient directly,
	// to allow for injection of a client in tests
	client              userClient
	promptEnableCaching func() (bool, error)
}

func (l *login) directUserToURL(url string) {
	err := l.openURL(url)
	if err != nil {
		l.base.UI.Warn(fmt.Sprintf("Failed to open browser. Please visit %v in your browser", url))
	}
}

func (l *login) run(ctx context.Context) error {
	loginURLBase := l.base.RepoConfig.LoginURL()
	l.base.Logger.Debug(fmt.Sprintf("turbo v%v", l.base.TurboVersion))
	l.base.Logger.Debug(fmt.Sprintf("api url: %v", l.base.RemoteConfig.APIURL))
	l.base.Logger.Debug(fmt.Sprintf("login url: %v", loginURLBase))
	redirectURL := fmt.Sprintf("http://%v:%v", defaultHostname, defaultPort)
	loginURL := fmt.Sprintf("%v/turborepo/token?redirect_uri=%v", loginURLBase, redirectURL)
	l.base.UI.Info(util.Sprintf(">>> Opening browser to %v", loginURL))

	rootctx, cancel := signal.NotifyContext(ctx, os.Interrupt)
	defer cancel()

	var query url.Values
	oss, err := newOneShotServer(rootctx, func(w http.ResponseWriter, r *http.Request) {
		query = r.URL.Query()
		http.Redirect(w, r, loginURLBase+"/turborepo/success", http.StatusFound)
	}, defaultPort)
	if err != nil {
		return errors.Wrap(err, "failed to start local server")
	}

	s := ui.NewSpinner(os.Stdout)
	l.directUserToURL(loginURL)
	s.Start("Waiting for your authorization...")
	err = oss.Wait()
	if err != nil {
		return errors.Wrap(err, "failed to shut down local server")
	}
	// Stop the spinner before we return to ensure terminal is left in a good state
	s.Stop("")

	if err := l.base.UserConfig.SetToken(query.Get("token")); err != nil {
		return err
	}
	rawToken := query.Get("token")
	l.client.SetToken(rawToken)
	userResponse, err := l.client.GetUser()
	if err != nil {
		return errors.Wrap(err, "could not get user information")
	}
	l.base.UI.Info("")
	l.base.UI.Info(util.Sprintf("%s Turborepo CLI authorized for %s${RESET}", ui.Rainbow(">>> Success!"), userResponse.User.Email))
	l.base.UI.Info("")
	l.base.UI.Info(util.Sprintf("${CYAN}To connect to your Remote Cache. Run the following in the${RESET}"))
	l.base.UI.Info(util.Sprintf("${CYAN}root of any turborepo:${RESET}"))
	l.base.UI.Info("")
	l.base.UI.Info(util.Sprintf("  ${BOLD}npx turbo link${RESET}"))
	l.base.UI.Info("")
	return nil
}

func (l *login) loginSSO(ctx context.Context, ssoTeam string) error {
	redirectURL := fmt.Sprintf("http://%v:%v", defaultHostname, defaultPort)
	query := make(url.Values)
	query.Add("teamId", ssoTeam)
	query.Add("mode", "login")
	query.Add("next", redirectURL)
	loginURL := fmt.Sprintf("%v/api/auth/sso?%v", l.base.RepoConfig.LoginURL(), query.Encode())

	rootctx, cancel := signal.NotifyContext(ctx, os.Interrupt)
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
	l.directUserToURL(loginURL)
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
	verifiedUser, err := l.client.VerifySSOToken(verificationToken, tokenName)
	if err != nil {
		return errors.Wrap(err, "failed to verify SSO token")
	}

	l.client.SetToken(verifiedUser.Token)
	l.client.SetTeamID(verifiedUser.TeamID)
	userResponse, err := l.client.GetUser()
	if err != nil {
		return errors.Wrap(err, "could not get user information")
	}
	if err := l.base.UserConfig.SetToken(verifiedUser.Token); err != nil {
		return errors.Wrap(err, "failed to save auth token")
	}
	l.base.UI.Info("")
	l.base.UI.Info(util.Sprintf("%s Turborepo CLI authorized for %s${RESET}", ui.Rainbow(">>> Success!"), userResponse.User.Email))
	l.base.UI.Info("")
	if verifiedUser.TeamID != "" {
		err = l.verifyCachingEnabled(verifiedUser.TeamID)
		if err != nil {
			return err
		}
		err = l.base.RepoConfig.SetTeamID(verifiedUser.TeamID)
		if err != nil {
			return errors.Wrap(err, "failed to save teamId")
		}
		l.base.UI.Info(util.Sprintf("${CYAN}Remote Caching enabled for %s${RESET}", ssoTeam))
		l.base.UI.Info("")
		l.base.UI.Info("  Remote Caching shares your cached Turborepo task outputs and logs across")
		l.base.UI.Info("  all your team’s Vercel projects. It also can share outputs")
		l.base.UI.Info("  with other services that enable Remote Caching, like CI/CD systems.")
		l.base.UI.Info("  This results in faster build times and deployments for your team.")
		l.base.UI.Info(util.Sprintf("  For more info, see ${UNDERLINE}https://turborepo.org/docs/features/remote-caching${RESET}"))
		l.base.UI.Info("")
		l.base.UI.Info(util.Sprintf("${GREY}To disable Remote Caching, run `npx turbo unlink`${RESET}"))
	} else {

		l.base.UI.Info(util.Sprintf("${CYAN}To connect to your Remote Cache. Run the following in the${RESET}"))
		l.base.UI.Info(util.Sprintf("${CYAN}root of any turborepo:${RESET}"))
		l.base.UI.Info("")
		l.base.UI.Info(util.Sprintf("  ${BOLD}npx turbo link${RESET}"))
	}
	l.base.UI.Info("")
	return nil
}

func (l *login) verifyCachingEnabled(teamID string) error {
	cachingStatus, err := l.client.GetCachingStatus()
	if err != nil {
		return err
	}
	switch cachingStatus {
	case util.CachingStatusDisabled:
		team, err := l.client.GetTeam(teamID)
		if err != nil {
			return err
		} else if team == nil {
			return fmt.Errorf("unable to find team %v", teamID)
		}
		if team.IsOwner() {
			shouldEnable, err := l.promptEnableCaching()
			if err != nil {
				return err
			}
			if shouldEnable {
				url := fmt.Sprintf("https://vercel.com/teams/%v/settings/billing", team.Slug)
				l.base.UI.Info(fmt.Sprintf("Visit %v in your browser to enable Remote Caching", url))
				l.directUserToURL(url)
				return errTryAfterEnable
			}
		}
		return errNeedCachingEnabled
	case util.CachingStatusOverLimit:
		return errOverage
	case util.CachingStatusPaused:
		return errPaused
	case util.CachingStatusEnabled:
	default:
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
