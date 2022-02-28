package auth

import (
	"context"
	"fmt"
	"net/http"
	"net/url"
	"os"

	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/cmdutil"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"
	"github.com/vercel/turborepo/cli/internal/util/browser"
)

const (
	DEFAULT_HOSTNAME = "127.0.0.1"
	DEFAULT_PORT     = 9789
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

			redirectUrl := fmt.Sprintf("http://%v:%v", DEFAULT_HOSTNAME, DEFAULT_PORT)
			loginUrl := fmt.Sprintf("%v/turborepo/token?redirect_uri=%v", ch.Config.LoginUrl, redirectUrl)
			ch.Logger.Printf(util.Sprintf(">>> Opening browser to %v", ch.Config.LoginUrl))
			s := ui.NewSpinner(os.Stdout)
			browser.OpenBrowser(loginUrl)
			s.Start("Waiting for your authorization...")

			var query url.Values
			ctx, cancel := context.WithCancel(context.Background())
			fmt.Println(query.Encode())
			http.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
				query = r.URL.Query()
				http.Redirect(w, r, ch.Config.LoginUrl+"/turborepo/success", http.StatusFound)
				cancel()
			})

			srv := &http.Server{Addr: DEFAULT_HOSTNAME + ":" + fmt.Sprint(DEFAULT_PORT)}
			go func() {
				if err := srv.ListenAndServe(); err != nil {
					if err != nil {
						ch.Logger.Printf(ch.LogError("could not activate device. Please try again: %w", err))
					}
				}
			}()
			<-ctx.Done()
			s.Stop("")

			config.WriteUserConfigFile(&config.TurborepoConfig{Token: query.Get("token")})
			rawToken = query.Get("token")
			ch.Config.ApiClient.SetToken(rawToken)

			userResponse, err := ch.Config.ApiClient.GetUser()
			if err != nil {
				return ch.LogError("could not get user information.\n%w", err)
			}

			ch.Logger.Printf("")
			ch.Logger.Printf(util.Sprintf("%s Turborepo CLI authorized for %s${RESET}", ui.Rainbow(">>> Success!"), userResponse.User.Email))
			ch.Logger.Printf("")
			ch.Logger.Printf(util.Sprintf("${CYAN}To connect to your Remote Cache. Run the following in the${RESET}"))
			ch.Logger.Printf(util.Sprintf("${CYAN}root of any turborepo:${RESET}"))
			ch.Logger.Printf("")
			ch.Logger.Printf(util.Sprintf("  ${BOLD}npx turbo link${RESET}"))
			ch.Logger.Printf("")

			return nil
		},
	}

	return cmd
}
