package login

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/AlecAivazis/survey/v2"
	"github.com/mitchellh/go-homedir"

	"github.com/pkg/errors"

	"github.com/vercel/turbo/cli/internal/client"
	"github.com/vercel/turbo/cli/internal/cmdutil"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbostate"
	"github.com/vercel/turbo/cli/internal/ui"
	"github.com/vercel/turbo/cli/internal/util"
	"github.com/vercel/turbo/cli/internal/util/browser"

	"github.com/fatih/color"
)

type link struct {
	base                *cmdutil.CmdBase
	modifyGitIgnore     bool
	apiClient           linkAPIClient // separate from base to allow testing
	promptSetup         func(location string) (bool, error)
	promptTeam          func(teams []string) (string, error)
	promptEnableCaching func() (bool, error)
	openBrowser         func(url string) error
}

type linkAPIClient interface {
	HasUser() bool
	GetTeams() (*client.TeamsResponse, error)
	GetUser() (*client.UserResponse, error)
	SetTeamID(teamID string)
	GetCachingStatus() (util.CachingStatus, error)
}

// ExecuteLink executes the `link` command.
func ExecuteLink(helper *cmdutil.Helper, args *turbostate.ParsedArgsFromRust) error {
	base, err := helper.GetCmdBase(args)
	if err != nil {
		return err
	}

	if args.TestRun {
		base.UI.Info("Link test run successful")
		return nil
	}

	link := &link{
		base:                base,
		modifyGitIgnore:     !args.Command.Link.DontModifyGitIgnore,
		apiClient:           base.APIClient,
		promptSetup:         promptSetup,
		promptTeam:          promptTeam,
		promptEnableCaching: promptEnableCaching,
		openBrowser:         browser.OpenBrowser,
	}
	err = link.run()
	if err != nil {
		if errors.Is(err, errUserCanceled) {
			base.UI.Info("Canceled. Turborepo not set up.")
		} else if errors.Is(err, errTryAfterEnable) || errors.Is(err, errNeedCachingEnabled) || errors.Is(err, errOverage) {
			base.UI.Info("Remote Caching not enabled. Please run 'turbo login' again after Remote Caching has been enabled")
		} else {
			link.logError(err)
		}
		return err
	}
	return nil
}

var errUserCanceled = errors.New("canceled")

func (l *link) run() error {
	dir, err := homedir.Dir()
	if err != nil {
		return fmt.Errorf("could not find home directory.\n%w", err)
	}
	l.base.UI.Info(">>> Remote Caching")
	l.base.UI.Info("")
	l.base.UI.Info("  Remote Caching shares your cached Turborepo task outputs and logs across")
	l.base.UI.Info("  all your team’s Vercel projects. It also can share outputs")
	l.base.UI.Info("  with other services that enable Remote Caching, like CI/CD systems.")
	l.base.UI.Info("  This results in faster build times and deployments for your team.")
	l.base.UI.Info(util.Sprintf("  For more info, see ${UNDERLINE}https://turbo.build/repo/docs/core-concepts/remote-caching${RESET}"))
	l.base.UI.Info("")
	currentDir, err := filepath.Abs(".")
	if err != nil {
		return fmt.Errorf("could figure out file path.\n%w", err)
	}
	repoLocation := strings.Replace(currentDir, dir, "~", 1)

	shouldSetup, err := l.promptSetup(repoLocation)
	if err != nil {
		return err
	}
	if !shouldSetup {
		return errUserCanceled
	}

	if !l.apiClient.HasUser() {
		return fmt.Errorf(util.Sprintf("User not found. Please login to Turborepo first by running ${BOLD}`npx turbo login`${RESET}."))
	}

	teamsResponse, err := l.apiClient.GetTeams()
	if err != nil {
		return fmt.Errorf("could not get team information.\n%w", err)
	}
	userResponse, err := l.apiClient.GetUser()
	if err != nil {
		return fmt.Errorf("could not get user information.\n%w", err)
	}

	// Gather team options
	teamOptions := make([]string, len(teamsResponse.Teams)+1)
	nameWithFallback := userResponse.User.Name
	if nameWithFallback == "" {
		nameWithFallback = userResponse.User.Username
	}
	teamOptions[0] = nameWithFallback
	for i, team := range teamsResponse.Teams {
		teamOptions[i+1] = team.Name
	}

	chosenTeamName, err := l.promptTeam(teamOptions)
	if err != nil {
		return err
	}
	if chosenTeamName == "" {
		return errUserCanceled
	}
	isUser := (chosenTeamName == userResponse.User.Name) || (chosenTeamName == userResponse.User.Username)
	var chosenTeam client.Team
	var teamID string
	if isUser {
		teamID = userResponse.User.ID
	} else {
		for _, team := range teamsResponse.Teams {
			if team.Name == chosenTeamName {
				chosenTeam = team
				break
			}
		}
		teamID = chosenTeam.ID
	}
	l.apiClient.SetTeamID(teamID)

	cachingStatus, err := l.apiClient.GetCachingStatus()
	if err != nil {
		return err
	}
	switch cachingStatus {
	case util.CachingStatusDisabled:
		if isUser || chosenTeam.IsOwner() {
			shouldEnable, err := l.promptEnableCaching()
			if err != nil {
				return err
			}
			if shouldEnable {
				var url string
				if isUser {
					url = "https://vercel.com/account/billing"
				} else {
					url = fmt.Sprintf("https://vercel.com/teams/%v/settings/billing", chosenTeam.Slug)
				}
				err = l.openBrowser(url)
				if err != nil {
					l.base.UI.Warn(fmt.Sprintf("Failed to open browser. Please visit %v to enable Remote Caching", url))
				} else {
					l.base.UI.Info(fmt.Sprintf("Visit %v in your browser to enable Remote Caching", url))
				}
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

	fs.EnsureDir(filepath.Join(".turbo", "config.json"))
	err = l.base.RepoConfig.SetTeamID(teamID)
	if err != nil {
		return fmt.Errorf("could not link current directory to team/user.\n%w", err)
	}

	if l.modifyGitIgnore {
		if err := l.addTurboToGitignore(); err != nil {
			return err
		}
	}

	l.base.UI.Info("")
	l.base.UI.Info(util.Sprintf("%s${RESET} Turborepo CLI authorized for ${BOLD}%s${RESET}", ui.Rainbow(">>> Success!"), chosenTeamName))
	l.base.UI.Info("")
	l.base.UI.Info(util.Sprintf("${GREY}To disable Remote Caching, run `npx turbo unlink`${RESET}"))
	l.base.UI.Info("")
	return nil
}

// logError logs an error and outputs it to the UI.
func (l *link) logError(err error) {
	l.base.Logger.Error(fmt.Sprintf("error: %v", err))
	l.base.UI.Error(fmt.Sprintf("%s%s", ui.ERROR_PREFIX, color.RedString(" %v", err)))
}

func promptSetup(location string) (bool, error) {
	shouldSetup := true
	err := survey.AskOne(
		&survey.Confirm{
			Default: true,
			Message: util.Sprintf("Would you like to enable Remote Caching for ${CYAN}${BOLD}\"%s\"${RESET}?", location),
		},
		&shouldSetup, survey.WithValidator(survey.Required),
		survey.WithIcons(func(icons *survey.IconSet) {
			// for more information on formatting the icons, see here: https://github.com/mgutz/ansi#style-format
			icons.Question.Format = "gray+hb"
		}))
	if err != nil {
		return false, err
	}
	return shouldSetup, nil
}

func (l *link) addTurboToGitignore() error {
	gitignorePath := l.base.RepoRoot.Join(".gitignore")

	if !gitignorePath.FileExists() {
		err := gitignorePath.WriteFile([]byte(".turbo\n"), 0644)
		if err != nil {
			return fmt.Errorf("could not create .gitignore.\n%w", err)
		}
		return nil
	}

	gitignoreBytes, err := gitignorePath.ReadFile()
	if err != nil {
		return fmt.Errorf("could not find or update .gitignore.\n%w", err)
	}

	hasTurbo := false
	gitignoreContents := string(gitignoreBytes)
	gitignoreLines := strings.Split(gitignoreContents, "\n")

	for _, line := range gitignoreLines {
		if strings.TrimSpace(line) == ".turbo" {
			hasTurbo = true
			break
		}
	}

	if !hasTurbo {
		gitignore, err := gitignorePath.OpenFile(os.O_APPEND|os.O_WRONLY, 0644)
		if err != nil {
			return fmt.Errorf("could not find or update .gitignore.\n%w", err)
		}

		// if the file doesn't end in a newline, we add one
		if !strings.HasSuffix(gitignoreContents, "\n") {
			if _, err := gitignore.WriteString("\n"); err != nil {
				return fmt.Errorf("could not find or update .gitignore.\n%w", err)
			}
		}

		if _, err := gitignore.WriteString(".turbo\n"); err != nil {
			return fmt.Errorf("could not find or update .gitignore.\n%w", err)
		}
	}

	return nil
}

func promptTeam(teams []string) (string, error) {
	chosenTeamName := ""
	err := survey.AskOne(
		&survey.Select{
			Message: "Which Vercel scope (and Remote Cache) do you want associate with this Turborepo? ",
			Options: teams,
		},
		&chosenTeamName,
		survey.WithValidator(survey.Required),
		survey.WithIcons(func(icons *survey.IconSet) {
			// for more information on formatting the icons, see here: https://github.com/mgutz/ansi#style-format
			icons.Question.Format = "gray+hb"
		}))
	if err != nil {
		return "", err
	}
	return chosenTeamName, nil
}
