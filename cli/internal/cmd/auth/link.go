package auth

import (
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/AlecAivazis/survey/v2"
	"github.com/mitchellh/go-homedir"
	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/client"
	"github.com/vercel/turborepo/cli/internal/cmdutil"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"
)

func LinkCmd(ch *cmdutil.Helper) *cobra.Command {
	var opts struct {
		noGitignore bool
	}

	cmd := &cobra.Command{
		Use:   "link",
		Short: "Link your local directory to a Vercel organization and enable remote caching",
		RunE: func(cmd *cobra.Command, args []string) error {
			shouldSetup := true
			dir, homeDirErr := homedir.Dir()
			if homeDirErr != nil {
				return ch.LogError("could not find home directory.\n%w", homeDirErr)
			}

			ch.Logger.Printf(">>> Remote Caching (beta)")
			ch.Logger.Printf("")
			ch.Logger.Printf("  Remote Caching shares your cached Turborepo task outputs and logs")
			ch.Logger.Printf("  across all your team’s Vercel projects. It also can share outputs")
			ch.Logger.Printf("  with other services that enable Remote Caching, like CI/CD systems.")
			ch.Logger.Printf("  This results in faster build times and deployments for your team.")
			ch.Logger.Printf(util.Sprintf("  For more info, see ${UNDERLINE}https://turborepo.org/docs/features/remote-caching${RESET}"))
			ch.Logger.Printf("")
			currentDir, fpErr := filepath.Abs(".")
			if fpErr != nil {
				return ch.LogError("could figure out file path.\n%w", fpErr)
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
				ch.Logger.Printf("> Aborted.")
				return nil
			}

			if ch.Config.Token == "" {
				return ch.LogError(util.Sprintf("user not found. Please login to Turborepo first by running ${BOLD}`npx turbo login`${RESET}."))
			}

			teamsResponse, err := ch.Config.ApiClient.GetTeams()
			if err != nil {
				return ch.LogError("could not get team information.\n%w", err)
			}
			userResponse, err := ch.Config.ApiClient.GetUser()
			if err != nil {
				return ch.LogError("could not get user information.\n%w", err)
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
				ch.Logger.Printf("Aborted. Turborepo not set up.")
				return nil
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
			fsErr := config.WriteConfigFile(filepath.Join(".turbo", "config.json"), &config.TurborepoConfig{
				TeamId: chosenTeam.ID,
				ApiUrl: ch.Config.ApiUrl,
			})
			if fsErr != nil {
				return ch.LogError("could not link current directory to team/user.\n%w", fsErr)
			}

			if !opts.noGitignore {
				fs.EnsureDir(".gitignore")
				_, gitIgnoreErr := exec.Command("sh", "-c", "grep -qxF '.turbo' .gitignore || echo '.turbo' >> .gitignore").CombinedOutput()
				if err != nil {
					return ch.LogError("could find or update .gitignore.\n%w", gitIgnoreErr)
				}
			}

			ch.Logger.Printf("")
			ch.Logger.Printf(util.Sprintf("%s${RESET} Turborepo CLI authorized for ${BOLD}%s${RESET}", ui.Rainbow(">>> Success!"), chosenTeam.Name))
			ch.Logger.Printf("")
			ch.Logger.Printf(util.Sprintf("${GREY}To disable Remote Caching, run `npx turbo unlink`${RESET}"))
			ch.Logger.Printf("")

			return nil
		},
	}

	cmd.Flags().BoolVarP(&opts.noGitignore, "no-gitignore", "n", false, "Do not create or modify .gitignore")

	return cmd
}
