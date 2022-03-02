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
	defaultHostname = "127.0.0.1"
	defaultPort     = 9789
)

func LoginCmd(ch *cmdutil.Helper) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "login",
		Short: "Login to your Vercel account",
		RunE: func(cmd *cobra.Command, args []string) error {
			var rawToken string

			ch.Config.Logger.Debug(fmt.Sprintf("turbo v%v", ch.Config.Version))
			ch.Config.Logger.Debug(fmt.Sprintf("api url: %v", ch.Config.ApiUrl))
			ch.Config.Logger.Debug(fmt.Sprintf("login url: %v", ch.Config.LoginUrl))

			redirectURL := fmt.Sprintf("http://%v:%v", defaultHostname, defaultPort)
			loginURL := fmt.Sprintf("%v/turborepo/token?redirect_uri=%v", ch.Config.LoginUrl, redirectURL)
			ch.Logger.Printf(">>> Opening browser to %v", ch.Config.LoginUrl)

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
				http.Redirect(w, r, ch.Config.LoginUrl+"/turborepo/success", http.StatusFound)
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
			err = browser.OpenBrowser(loginURL)
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

			rawToken = query.Get("token")
			config.WriteUserConfigFile(&config.TurborepoConfig{Token: rawToken})
			ch.Config.ApiClient.SetToken(rawToken)

			userResponse, err := ch.Config.ApiClient.GetUser()
			if err != nil {
				return errors.Wrap(err, "could not get user information")
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
		},
	}

	return cmd
}
