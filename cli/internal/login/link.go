package login

import (
	"fmt"
	"os/exec"
	"path/filepath"
	"strings"
	"turbo/internal/client"
	"turbo/internal/config"
	"turbo/internal/fs"
	"turbo/internal/ui"
	"turbo/internal/util"

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

// Synopsis of run command
func (c *LinkCommand) Synopsis() string {
	return "Link your local directory to a Vercel organization and enable remote caching."
}

// Help returns information about the `run` command
func (c *LinkCommand) Help() string {
	helpText := `
Usage: turbo link

  Link your local directory to a Vercel organization and enable remote caching.

Options:
  --help                 Show this screen.
  --no-gitignore         Do not create or modify .gitignore
                         (default false)
`
	return strings.TrimSpace(helpText)
}

// Run executes tasks in the monorepo
func (c *LinkCommand) Run(args []string) int {
	var dontModifyGitIgnore bool
	shouldSetup := true
	dir, homeDirErr := homedir.Dir()
	if homeDirErr != nil {
		c.logError(fmt.Errorf("could not find home directory.\n%w", homeDirErr))
		return 1
	}
	c.Ui.Info(fmt.Sprintf(">>> Remote Caching (beta)"))
	c.Ui.Info("")
	c.Ui.Info("  Remote Caching shares your cached Turborepo task outputs and logs across")
	c.Ui.Info("  with all of your team's Vercel projects and with any other")
	c.Ui.Info("  team members and/or services that also enable Remote Caching. This results")
	c.Ui.Info("  in a faster build time and faster deployment time for your team.")
	c.Ui.Info(util.Sprintf("  For more info, see ${UNDERLINE}https://turborepo.org/docs/features/remote-caching${RESET}"))
	c.Ui.Info("")
	currentDir, fpErr := filepath.Abs(".")
	if fpErr != nil {
		c.logError(fmt.Errorf("could figure out file path.\n%w", fpErr))
		return 1
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
		c.Ui.Info("> Aborted.")
		return 1
	}

	if c.Config.Token == "" {
		c.logError(fmt.Errorf(util.Sprintf("User not found. Please login to Vercel first by running ${BOLD}`npx vercel login`${RESET}.")))
		return 1
	}

	teamsResponse, err := c.Config.ApiClient.GetTeams()
	if err != nil {
		c.logError(fmt.Errorf("could not get team information.\n%w", err))
		return 1
	}
	userResponse, err := c.Config.ApiClient.GetUser()
	if err != nil {
		c.logError(fmt.Errorf("could not get user information.\n%w", err))
		return 1
	}

	var chosenTeam client.Team

	teamOptions := make([]string, len(teamsResponse.Teams))

	// Gather team options
	for i, team := range teamsResponse.Teams {
		teamOptions[i] = team.Name
	}

	var chosenTeamName string
	survey.AskOne(
		&survey.Select{
			Message: "Which Vercel scope (and Remote Cache) do you want associate with this Turborepo? ",
			Options: append([]string{userResponse.User.Name}, teamOptions...),
		},
		&chosenTeamName,
		survey.WithValidator(survey.Required),
		survey.WithIcons(func(icons *survey.IconSet) {
			// for more information on formatting the icons, see here: https://github.com/mgutz/ansi#style-format
			icons.Question.Format = "gray+hb"
		}))

	if chosenTeamName == "" {
		c.Ui.Info("Aborted. Turborepo not set up.")
		return 1
	} else if chosenTeamName == userResponse.User.Name {
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
	fsErr := config.WriteConfigFile(filepath.Join(".turbo", "config.json"), &config.TurborepoConfig{
		TeamId: chosenTeam.ID,
		ApiUrl: c.Config.ApiUrl,
	})
	if fsErr != nil {
		c.logError(fmt.Errorf("could not link current directory to team/user.\n%w", fsErr))
		return 1
	}

	if !dontModifyGitIgnore {
		fs.EnsureDir(".gitignore")
		_, gitIgnoreErr := exec.Command("sh", "-c", "grep -qxF '.turbo' .gitignore || echo '.turbo' >> .gitignore").CombinedOutput()
		if err != nil {
			c.logError(fmt.Errorf("could find or update .gitignore.\n%w", gitIgnoreErr))
			return 1
		}
	}

	c.Ui.Info("")
	c.Ui.Info(util.Sprintf("%s${RESET} Turborepo CLI authorized for ${BOLD}%s${RESET}", ui.Rainbow(">>> Success!"), chosenTeam.Name))
	c.Ui.Info("")
	c.Ui.Info(util.Sprintf("${GREY}To disable Remote Caching, run `npx turbo unlink`${RESET}"))
	c.Ui.Info("")
	return 0
}

// logError logs an error and outputs it to the UI.
func (c *LinkCommand) logError(err error) {
	c.Config.Logger.Error("error", err)
	c.Ui.Error(fmt.Sprintf("%s%s", ui.ERROR_PREFIX, color.RedString(" %v", err)))
}
