package login

import (
	"context"
	"fmt"
	"log"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"runtime"
	"strings"
	"turbo/internal/config"
	"turbo/internal/ui"
	"turbo/internal/util"

	"github.com/fatih/color"
	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
)

// LoginCommand is a Command implementation allows the user to login to turbo
type LoginCommand struct {
	Config *config.Config
	Ui     *cli.ColoredUi
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

const DEFAULT_HOSTNAME = "127.0.0.1"
const DEFAULT_PORT = 9789

// Run logs into the api with PKCE and writes the token to turbo user config directory
func (c *LoginCommand) Run(args []string) int {
	var rawToken string
	c.Config.Logger.Debug(fmt.Sprintf("turbo v%v", c.Config.TurboVersion))
	c.Config.Logger.Debug(fmt.Sprintf("api url: %v", c.Config.ApiUrl))
	c.Config.Logger.Debug(fmt.Sprintf("login url: %v", c.Config.LoginUrl))
	redirectUrl := fmt.Sprintf("http://%v:%v", DEFAULT_HOSTNAME, DEFAULT_PORT)
	loginUrl := fmt.Sprintf("%v/turborepo/token?redirect_uri=%v", c.Config.LoginUrl, redirectUrl)
	c.Ui.Info(util.Sprintf(">>> Opening browser to %v", c.Config.LoginUrl))
	s := ui.NewSpinner(os.Stdout)
	openbrowser(loginUrl)
	s.Start("Waiting for your authorization...")

	var query url.Values
	ctx, cancel := context.WithCancel(context.Background())
	fmt.Println(query.Encode())
	http.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		query = r.URL.Query()
		http.Redirect(w, r, c.Config.LoginUrl+"/turborepo/success", http.StatusFound)
		cancel()
	})

	srv := &http.Server{Addr: "127.0.0.1:9789"}
	go func() {
		if err := srv.ListenAndServe(); err != nil {
			if err != nil {
				c.logError(c.Config.Logger, "", fmt.Errorf("Could not activate device. Please try again: %w", err))
			}
		}
	}()
	<-ctx.Done()
	s.Stop("")
	config.WriteUserConfigFile(&config.TurborepoConfig{Token: query.Get("token")})
	rawToken = query.Get("token")
	c.Config.ApiClient.SetToken(rawToken)
	userResponse, err := c.Config.ApiClient.GetUser()
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("could not get user information.\n: %w", err))
		return 1
	}
	c.Ui.Info("")
	c.Ui.Info(util.Sprintf("%s Turborepo CLI authorized for %s${RESET}", ui.Rainbow(">>> Success!"), userResponse.User.Email))
	c.Ui.Info("")
	c.Ui.Info(util.Sprintf("${CYAN}To connect to your Remote Cache. Run the following in the${RESET}"))
	c.Ui.Info(util.Sprintf("${CYAN}root of any turborepo:${RESET}"))
	c.Ui.Info("")
	c.Ui.Info(util.Sprintf("  ${BOLD}npx turbo link${RESET}"))
	c.Ui.Info("")
	return 0
}

// logError logs an error and outputs it to the UI.
func (c *LoginCommand) logError(log hclog.Logger, prefix string, err error) {
	log.Error(prefix, "error", err)

	if prefix != "" {
		prefix += ": "
	}

	c.Ui.Error(fmt.Sprintf("%s%s%s", ui.ERROR_PREFIX, prefix, color.RedString(" %v", err)))
}

// logError logs an error and outputs it to the UI.
func (c *LoginCommand) logWarning(log hclog.Logger, prefix string, err error) {
	log.Warn(prefix, "warning", err)

	if prefix != "" {
		prefix = " " + prefix + ": "
	}

	c.Ui.Error(fmt.Sprintf("%s%s%s", ui.WARNING_PREFIX, prefix, color.YellowString(" %v", err)))
}

// logError logs an error and outputs it to the UI.
func (c *LoginCommand) logFatal(log hclog.Logger, prefix string, err error) {
	log.Error(prefix, "error", err)

	if prefix != "" {
		prefix += ": "
	}

	c.Ui.Error(fmt.Sprintf("%s%s%s", ui.ERROR_PREFIX, prefix, color.RedString(" %v", err)))
	os.Exit(1)
}

func openbrowser(url string) {
	var err error

	switch runtime.GOOS {
	case "linux":
		err = exec.Command("xdg-open", url).Start()
	case "windows":
		err = exec.Command("rundll32", "url.dll,FileProtocolHandler", url).Start()
	case "darwin":
		err = exec.Command("open", url).Start()
	default:
		err = fmt.Errorf("unsupported platform")
	}
	if err != nil {
		log.Fatal(err)
	}

}
