package recent

import (
	"encoding/json"
	"fmt"
	"os"
	"text/tabwriter"
	"time"

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
	var outputJSON bool
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
			if outputJSON {
				return renderJSON(terminal, summary)
			}
			return renderText(terminal, summary)
		},
	}
	cmd.Flags().BoolVar(&outputJSON, "json", false, "Output summary in JSON format")
	return cmd
}

func renderText(terminal cli.Ui, summary map[string]interface{}) error {
	terminal.Output("")
	terminal.Info(util.Sprintf("${CYAN}${BOLD}turbo run session %v${RESET}", summary["sessionId"]))
	tw := tabwriter.NewWriter(os.Stdout, 0, 0, 4, ' ', 0)
	fmt.Fprintf(tw, "Command\t%v\n", summary["command"])
	start := time.UnixMilli(int64(summary["startedAt"].(float64)))
	startString := start.Format(time.RFC3339)
	end := time.UnixMilli(int64(summary["endedAt"].(float64)))
	duration := end.Sub(start)
	fmt.Fprintf(tw, "Started\t%v (%v)\n", startString, duration)
	if err := tw.Flush(); err != nil {
		return err
	}
	terminal.Info(util.Sprintf("Entrypoint Packages:"))
	entrypoints := summary["entrypointPackages"].([]interface{})
	for _, pkg := range entrypoints {
		terminal.Info(util.Sprintf("${GREY}\t%v${RESET}", pkg))
	}
	terminal.Info(util.Sprintf("Entrypoint Tasks:"))
	targets := summary["targets"].([]interface{})
	for _, target := range targets {
		terminal.Info(util.Sprintf("${GREY}\t%v${RESET}", target))
	}
	terminal.Info(util.Sprintf("Tasks:"))
	tasks := summary["tasks"].(map[string]interface{})
	for taskID, taskSummary := range tasks {
		ts := taskSummary.(map[string]interface{})
		tw = tabwriter.NewWriter(os.Stdout, 0, 0, 1, ' ', 0)
		terminal.Info(util.Sprintf("${BOLD}%s${RESET}", taskID))
		fmt.Fprintln(tw, util.Sprintf("  ${GREY}Hash\t=\t%v${RESET}", ts["taskHash"]))
		fmt.Fprintln(tw, util.Sprintf("  ${GREY}Status\t=\t%v${RESET}", ts["status"]))
		started := time.UnixMilli(int64(ts["startedAt"].(float64)))
		duration := time.Duration(ts["durationMs"].(float64))
		fmt.Fprintln(tw, util.Sprintf("  ${GREY}Started\t=\t%v (%v)${RESET}", started.Format(time.RFC3339), duration))
		if err := tw.Flush(); err != nil {
			return err
		}
	}
	terminal.Output("")
	return nil
}

func renderJSON(terminal cli.Ui, summary map[string]interface{}) error {
	rendered, err := json.MarshalIndent(summary, "", "\t")
	if err != nil {
		return err
	}
	terminal.Output(string(rendered))
	return nil
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
