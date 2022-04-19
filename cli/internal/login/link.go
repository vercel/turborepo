package login

import (
	"fmt"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/pkg/errors"
	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/client"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"

	"github.com/AlecAivazis/survey/v2"
	"github.com/fatih/color"
	"github.com/mitchellh/cli"
	"github.com/mitchellh/go-homedir"
)

// LinkCommand is a Command implementation allows the user to link your local directory to a Turbrepo
type LinkCommand struct {
	Config *config.Config
	Ui     *cli.ColoredUi
}

type link struct {
	config          *config.Config
	ui              cli.Ui
	modifyGitIgnore bool
}

func getCmd(config *config.Config, ui cli.Ui) *cobra.Command {
	var dontModifyGitIgnore bool
	cmd := &cobra.Command{
		Use:   "turbo link",
		Short: "Link your local directory to a Vercel organization and enable remote caching.",
		RunE: func(cmd *cobra.Command, args []string) error {
			link := &link{
				config:          config,
				ui:              ui,
				modifyGitIgnore: !dontModifyGitIgnore,
			}
			return link.run()
		},
	}
	cmd.Flags().BoolVar(&dontModifyGitIgnore, "no-gitignore", false, "Do not create or modify .gitignore (default false)")
	return cmd
}

// Synopsis of link command
func (c *LinkCommand) Synopsis() string {
	cmd := getCmd(c.Config, c.Ui)
	return cmd.Short
}

// Help returns information about the `link` command
func (c *LinkCommand) Help() string {
	cmd := getCmd(c.Config, c.Ui)
	return util.HelpForCobraCmd(cmd)
}

// Run links a local directory to a Vercel organization and enables remote caching
func (c *LinkCommand) Run(args []string) int {
	cmd := getCmd(c.Config, c.Ui)
	cmd.SetArgs(args)
	err := cmd.Execute()
	if err != nil {
		if errors.Is(err, errUserCancelled) {
			c.Ui.Info("Cancelled. Turborepo not set up.")
		} else {
			c.logError(err)
		}
		return 1
	}
	return 0
}

var errUserCancelled = errors.New("cancelled")

func (l *link) run() error {
	shouldSetup := true
	dir, err := homedir.Dir()
	if err != nil {
		return fmt.Errorf("could not find home directory.\n%w", err)
	}
	l.ui.Info(">>> Remote Caching (beta)")
	l.ui.Info("")
	l.ui.Info("  Remote Caching shares your cached Turborepo task outputs and logs across")
	l.ui.Info("  all your team’s Vercel projects. It also can share outputs")
	l.ui.Info("  with other services that enable Remote Caching, like CI/CD systems.")
	l.ui.Info("  This results in faster build times and deployments for your team.")
	l.ui.Info(util.Sprintf("  For more info, see ${UNDERLINE}https://turborepo.org/docs/features/remote-caching${RESET}"))
	l.ui.Info("")
	currentDir, err := filepath.Abs(".")
	if err != nil {
		return fmt.Errorf("could figure out file path.\n%w", err)
	}

	survey.AskOne(
		&survey.Confirm{
			Default: true,
			Message: util.Sprintf("Would you like to enable Remote Caching for ${CYAN}${BOLD}\"%s\"${RESET}?", strings.Replace(currentDir, dir, "~", 1)),
		},
		&shouldSetup, survey.WithValidator(survey.Required),
		survey.WithIcons(func(icons *survey.IconSet) {
			// for more information on formatting the icons, see here: https://github.com/mgutz/ansi#style-format
			icons.Question.Format = "gray+hb"
		}))

	if !shouldSetup {
		return errUserCancelled
	}

	if l.config.Token == "" {
		return fmt.Errorf(util.Sprintf("User not found. Please login to Turborepo first by running ${BOLD}`npx turbo login`${RESET}."))
	}

	teamsResponse, err := l.config.ApiClient.GetTeams()
	if err != nil {
		return fmt.Errorf("could not get team information.\n%w", err)
	}
	userResponse, err := l.config.ApiClient.GetUser()
	if err != nil {
		return fmt.Errorf("could not get user information.\n%w", err)
	}

	var chosenTeam client.Team

	teamOptions := make([]string, len(teamsResponse.Teams))

	// Gather team options
	for i, team := range teamsResponse.Teams {
		teamOptions[i] = team.Name
	}

	var chosenTeamName string
	nameWithFallback := userResponse.User.Name
	if nameWithFallback == "" {
		nameWithFallback = userResponse.User.Username
	}
	survey.AskOne(
		&survey.Select{
			Message: "Which Vercel scope (and Remote Cache) do you want associate with this Turborepo? ",
			Options: append([]string{nameWithFallback}, teamOptions...),
		},
		&chosenTeamName,
		survey.WithValidator(survey.Required),
		survey.WithIcons(func(icons *survey.IconSet) {
			// for more information on formatting the icons, see here: https://github.com/mgutz/ansi#style-format
			icons.Question.Format = "gray+hb"
		}))

	if chosenTeamName == "" {
		return errUserCancelled
	} else if (chosenTeamName == userResponse.User.Name) || (chosenTeamName == userResponse.User.Username) {
		chosenTeam = client.Team{
			ID:   userResponse.User.ID,
			Name: userResponse.User.Name,
			Slug: userResponse.User.Username,
		}
	} else {
		for _, team := range teamsResponse.Teams {
			if team.Name == chosenTeamName {
				chosenTeam = team
				break
			}
		}
	}
	fs.EnsureDir(filepath.Join(".turbo", "config.json"))
	err = config.WriteRepoConfigFile(&config.TurborepoConfig{
		TeamId: chosenTeam.ID,
		ApiUrl: l.config.ApiUrl,
	})
	if err != nil {
		return fmt.Errorf("could not link current directory to team/user.\n%w", err)
	}

	if l.modifyGitIgnore {
		fs.EnsureDir(".gitignore")
		_, gitIgnoreErr := exec.Command("sh", "-c", "grep -qxF '.turbo' .gitignore || echo '.turbo' >> .gitignore").CombinedOutput()
		if err != nil {
			return fmt.Errorf("could find or update .gitignore.\n%w", gitIgnoreErr)
		}
	}

	l.ui.Info("")
	l.ui.Info(util.Sprintf("%s${RESET} Turborepo CLI authorized for ${BOLD}%s${RESET}", ui.Rainbow(">>> Success!"), chosenTeamName))
	l.ui.Info("")
	l.ui.Info(util.Sprintf("${GREY}To disable Remote Caching, run `npx turbo unlink`${RESET}"))
	l.ui.Info("")
	return nil
}

// logError logs an error and outputs it to the UI.
func (c *LinkCommand) logError(err error) {
	c.Config.Logger.Error("error", err)
	c.Ui.Error(fmt.Sprintf("%s%s", ui.ERROR_PREFIX, color.RedString(" %v", err)))
}
