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

	"github.com/AlecAivazis/survey/v2"
	"github.com/fatih/color"
	"github.com/mitchellh/cli"
	"github.com/mitchellh/go-homedir"
)

// LinkCommand is a Command implementation allows the user to link your local directory to a Project
type LinkCommand struct {
	Config *config.Config
	Ui     *cli.ColoredUi
}

// Synopsis of run command
func (c *LinkCommand) Synopsis() string {
	return "Link your local directory to a Vercel.com Organization"
}

// Help returns information about the `run` command
func (c *LinkCommand) Help() string {
	helpText := `
Usage: turbo link

  Link your local directory to a Vercel.com Organization. This will enable remote caching.

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
	c.Ui.Info(ui.Dim("Turborepo CLI"))
	shouldSetup := true
	dir, homeDirErr := homedir.Dir()
	if homeDirErr != nil {
		c.logError(fmt.Errorf("Could not find home directory.\n%w", homeDirErr))
		return 1
	}

	currentDir, fpErr := filepath.Abs(".")
	if fpErr != nil {
		c.logError(fmt.Errorf("Could figure out file path.\n%w", fpErr))
		return 1
	}

	survey.AskOne(
		&survey.Confirm{
			Default: true,
			Message: sprintf("Set up ${CYAN}${BOLD}\"%s\"${RESET}?", strings.Replace(currentDir, dir, "~", 1)),
		},
		&shouldSetup, survey.WithValidator(survey.Required),
		survey.WithIcons(func(icons *survey.IconSet) {
			// for more information on formatting the icons, see here: https://github.com/mgutz/ansi#style-format
			icons.Question.Format = "gray+hb"
		}))

	if !shouldSetup {
		c.Ui.Info("Aborted. Turborepo not set up.")
		return 1
	}

	teamsResponse, err := c.Config.ApiClient.GetTeams()
	if err != nil {
		c.logError(fmt.Errorf("could not get team information.\n%w", err))
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
			Message: "Which team scope should contain your turborepo?",
			Options: teamOptions,
		},
		&chosenTeamName,
		survey.WithValidator(survey.Required),
		survey.WithIcons(func(icons *survey.IconSet) {
			// for more information on formatting the icons, see here: https://github.com/mgutz/ansi#style-format
			icons.Question.Format = "gray+hb"
		}))

	for _, team := range teamsResponse.Teams {
		if team.Name == chosenTeamName {
			chosenTeam = team
			break
		}
	}
	fs.EnsureDir(filepath.Join(".turbo", "config.json"))
	fsErr := config.WriteConfigFile(filepath.Join(".turbo", "config.json"), &config.TurborepoConfig{
		TeamId: chosenTeam.ID,
		ApiUrl: c.Config.ApiUrl,
	})
	if fsErr != nil {
		c.logError(fmt.Errorf("Could not link current directory to team.\n%w", fsErr))
		return 1
	}

	if !dontModifyGitIgnore {
		fs.EnsureDir(".gitignore")
		_, gitIgnoreErr := exec.Command("sh", "-c", "grep -qxF '.turbo' .gitignore || echo '.turbo' >> .gitignore").CombinedOutput()
		if err != nil {
			c.logError(fmt.Errorf("Could find or update .gitignore.\n%w", gitIgnoreErr))
			return 1
		}
	}

	c.Ui.Info("")
	c.Ui.Info(sprintf("${GREEN}✓${RESET} Directory linked to ${BOLD}%s${RESET}", chosenTeam.Slug))
	c.Ui.Info(sprintf("${GREEN}✓${RESET} Remote caching is now enabled"))

	return 0
}

// logError logs an error and outputs it to the UI.
func (c *LinkCommand) logError(err error) {
	c.Config.Logger.Error("error", err)
	c.Ui.Error(fmt.Sprintf("%s%s", ui.ERROR_PREFIX, color.RedString(" %v", err)))
}
