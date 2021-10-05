package login

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"turbo/internal/config"
	"turbo/internal/fs"
	"turbo/internal/graphql"
	"turbo/internal/ui"

	"github.com/AlecAivazis/survey/v2"
	"github.com/fatih/color"
	"github.com/gosimple/slug"
	"github.com/hashicorp/go-hclog"
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
	return "Link your local directory to a Turborepo.com Project"
}

// Help returns information about the `run` command
func (c *LinkCommand) Help() string {
	helpText := `
Usage: turbo link

  Link your local directory to a Turborepo.com Project

Options:
  --help                 Show this screen.
  --no-gitignore         Do not create or modify .gitignore
                         (default false)
`
	return strings.TrimSpace(helpText)
}

// Run executes tasks in the monorepo
func (c *LinkCommand) Run(args []string) int {
	var rawToken string
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
		c.Ui.Info("Aborted. Project not set up.")
		return 1
	}

	userConfig, err := config.ReadUserConfigFile()
	if rawToken == "" {
		rawToken = userConfig.Token
	}
	req, err := graphql.NewGetViewerRequest(c.Config.ApiUrl)
	req.Header.Set("Authorization", "Bearer "+rawToken)
	if err != nil {
		c.logError(fmt.Errorf("Could not create internal API client. Please try again\n%w", err))
		return 1
	}
	gqlClient := graphql.NewClient(c.Config.ApiUrl)
	res, resErr := req.Execute(gqlClient.Client)
	if resErr != nil {
		c.logError(fmt.Errorf("Could not get user. Please try logging in again.\n%w", resErr))
		return 1
	}

	chosenTeam := struct {
		ID       string `json:"id"`
		Name     string `json:"name"`
		Slug     string `json:"slug"`
		PaidPlan string `json:"paidPlan"`
	}{}
	chosenProject := struct {
		ID        string `json:"id"`
		Slug      string `json:"slug"`
		CreatedAt string `json:"createdAt"`
		UpdatedAt string `json:"updatedAt"`
	}{}

	// if len(*res.Viewer.Teams.Nodes) > 0 {
	teamOptions := make([]string, len(*res.Viewer.Teams.Nodes))

	// Gather team options
	for i, team := range *res.Viewer.Teams.Nodes {
		teamOptions[i] = team.Name
	}

	var chosenTeamName string
	survey.AskOne(
		&survey.Select{
			Message: "Which team scope should contain your project?",
			Options: teamOptions,
		},
		&chosenTeamName,
		survey.WithValidator(survey.Required),
		survey.WithIcons(func(icons *survey.IconSet) {
			// for more information on formatting the icons, see here: https://github.com/mgutz/ansi#style-format
			icons.Question.Format = "gray+hb"
		}))

	for _, team := range *res.Viewer.Teams.Nodes {
		if team.Name == chosenTeamName {
			chosenTeam = team
			break
		}
	}
	var linkToExisting bool
	survey.AskOne(
		&survey.Confirm{
			Default: false,
			Message: "Link to an existing project?",
		},
		&linkToExisting,
		survey.WithValidator(survey.Required),
		survey.WithIcons(func(icons *survey.IconSet) {
			// for more information on formatting the icons, see here: https://github.com/mgutz/ansi#style-format
			icons.Question.Format = "gray+hb"
		}))

	if linkToExisting == true {
		var chosenProjectSlug string
		for chosenProject.ID == "" {
			survey.AskOne(
				&survey.Input{
					Message: "What's the name of your existing project?",
				},
				&chosenProjectSlug,
				survey.WithValidator(survey.Required),
				survey.WithIcons(func(icons *survey.IconSet) {
					// for more information on formatting the icons, see here: https://github.com/mgutz/ansi#style-format
					icons.Question.Format = "gray+hb"
				}))

			req, err := graphql.NewGetProjectRequest(c.Config.ApiUrl, &graphql.GetProjectVariables{
				TeamId: (*graphql.String)(&chosenTeam.ID),
				Slug:   (*graphql.String)(&chosenProjectSlug),
			})
			req.Header.Set("Authorization", "Bearer "+rawToken)
			if err != nil {
				c.logError(fmt.Errorf("Could not find project.\n%w", err))
				return 1
			}
			gqlClient := graphql.NewClient(c.Config.ApiUrl)
			res, resErr := req.Execute(gqlClient.Client)
			if resErr != nil {
				c.logError(fmt.Errorf("Could not find project.\n%w", resErr))
				return 1
			}
			if res.Project.ID == "" {
				var shouldCreateNewProjectAnyways bool
				survey.AskOne(
					&survey.Confirm{
						Message: fmt.Sprintf("That project wasn't found. Do you want to create a new project called `%v` anyways?", chosenProjectSlug),
						Default: true,
					},
					&shouldCreateNewProjectAnyways,
					survey.WithValidator(survey.Required),
					survey.WithIcons(func(icons *survey.IconSet) {
						// for more information on formatting the icons, see here: https://github.com/mgutz/ansi#style-format
						icons.Question.Format = "gray+hb"
					}))
				if shouldCreateNewProjectAnyways {
					req, err := graphql.NewCreateProjectRequest(c.Config.ApiUrl, &graphql.CreateProjectVariables{
						TeamId: graphql.String(chosenTeam.ID),
						Slug:   graphql.String(chosenProjectSlug),
					})
					req.Header.Set("Authorization", "Bearer "+rawToken)
					if err != nil {
						c.logError(fmt.Errorf("Could not create project.\n%w", err))
						return 1
					}
					gqlClient := graphql.NewClient(c.Config.ApiUrl)
					res, resErr := req.Execute(gqlClient.Client)
					if resErr != nil {
						c.logError(fmt.Errorf("Could not create project.\n%w", resErr))
						return 1
					}
					chosenProject = res.CreateProject
					break
				}
			}
			chosenProject = res.Project
		}
	} else {
		var chosenProjectSlug string
		pkgJson, pkgJsonErr := fs.ReadPackageJSON("package.json")
		if pkgJsonErr != nil {
		} // do nothing
		for chosenProject.ID == "" {
			survey.AskOne(
				&survey.Input{
					Message: "What's your project's name?",
					Default: slug.Make(pkgJson.Name),
				},
				&chosenProjectSlug,
				survey.WithValidator(survey.Required),
				survey.WithIcons(func(icons *survey.IconSet) {
					// for more information on formatting the icons, see here: https://github.com/mgutz/ansi#style-format
					icons.Question.Format = "gray+hb"
				}))

			req, err := graphql.NewCreateProjectRequest(c.Config.ApiUrl, &graphql.CreateProjectVariables{
				TeamId: graphql.String(chosenTeam.ID),
				Slug:   graphql.String(chosenProjectSlug),
			})
			req.Header.Set("Authorization", "Bearer "+rawToken)
			if err != nil {
				c.logError(fmt.Errorf("Could not create project.\n%w", err))
				return 1
			}
			gqlClient := graphql.NewClient(c.Config.ApiUrl)
			res, resErr := req.Execute(gqlClient.Client)
			if resErr != nil {
				c.logError(fmt.Errorf("Could not create project.\n%w", resErr))
				return 1
			}
			chosenProject = res.CreateProject
		}
	}
	fs.EnsureDir(".turbo/config.json")
	fsErr := config.WriteConfigFile(".turbo/config.json", &config.TurborepoConfig{
		ProjectId: chosenProject.ID,
		TeamId:    chosenTeam.ID,
		ApiUrl:    c.Config.ApiUrl,
	})
	if fsErr != nil {
		c.logError(fmt.Errorf("Could not link directory to team and project.\n%w", fsErr))
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
	c.Ui.Info(sprintf("${GREEN}✓${RESET} Directory linked to ${BOLD}%s/%s${RESET}", chosenTeam.Slug, chosenProject.Slug))

	return 0
}

// logError logs an error and outputs it to the UI.
func (c *LinkCommand) logError(err error) {
	c.Config.Logger.Error("error", err)
	c.Ui.Error(fmt.Sprintf("%s%s", ui.ERROR_PREFIX, color.RedString(" %v", err)))
}

// logError logs an error and outputs it to the UI.
func (c *LinkCommand) logWarning(log hclog.Logger, prefix string, err error) {
	log.Warn(prefix, "warning", err)

	if prefix != "" {
		prefix = " " + prefix + ": "
	}

	c.Ui.Error(fmt.Sprintf("%s%s%s", ui.WARNING_PREFIX, prefix, color.YellowString(" %v", err)))
}

// logError logs an error and outputs it to the UI.
func (c *LinkCommand) logFatal(log hclog.Logger, prefix string, err error) {
	log.Error(prefix, "error", err)

	if prefix != "" {
		prefix += ": "
	}

	c.Ui.Error(fmt.Sprintf("%s%s%s", ui.ERROR_PREFIX, prefix, color.RedString(" %v", err)))
	os.Exit(1)
}
