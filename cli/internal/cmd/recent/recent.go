package recent

import (
	"encoding/json"
	"os"

	"github.com/mitchellh/cli"
	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/util"
)

// Command is a temporary cli.Command wrapper for the recent command
type Command struct {
	Config *config.Config
	UI     cli.Ui
}

// Synopsis implements cli.Command.Synopsis
func (c *Command) Synopsis() string {
	return getCmd(c.Config, c.UI).Short
}

// Help implements cli.Command.Help
func (c *Command) Help() string {
	return util.HelpForCobraCmd(getCmd(c.Config, c.UI))
}

// Run implements cli.Command.Run
func (c *Command) Run(args []string) int {
	cmd := getCmd(c.Config, c.UI)
	cmd.SetArgs(args)
	err := cmd.Execute()
	if err != nil {
		return 1
	}
	return 0
}

func getCmd(config *config.Config, terminal cli.Ui) *cobra.Command {
	cmd := &cobra.Command{
		Use:           "turbo recent",
		Short:         "Summarizes the most recent run",
		SilenceUsage:  true,
		SilenceErrors: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			summaryDir := config.Cwd.Join(".turbo", "runs")
			summary, err := findMostRecentSummary(summaryDir)
			if err != nil {
				if os.IsNotExist(err) {
					terminal.Warn("No recent turbo runs found")
					return nil
				}
				return err
			}
			rendered, err := json.MarshalIndent(summary, "", "\t")
			if err != nil {
				return err
			}
			terminal.Output(string(rendered))
			return nil
		},
	}
	return cmd
}

func findMostRecentSummary(summaryDir fs.AbsolutePath) (map[string]interface{}, error) {
	entries, err := summaryDir.ReadDir()
	if err != nil {
		return nil, err
	} else if len(entries) == 0 {
		return nil, os.ErrNotExist
	}
	max := ""
	for _, entry := range entries {
		if entry.Name() > max {
			max = entry.Name()
		}
	}
	if max == "" {
		return nil, os.ErrNotExist
	}
	summaryPath := summaryDir.Join(max)
	raw, err := summaryPath.ReadFile()
	if err != nil {
		return nil, err
	}
	summary := make(map[string]interface{})
	if err := json.Unmarshal(raw, &summary); err != nil {
		return nil, err
	}
	return summary, nil
}
