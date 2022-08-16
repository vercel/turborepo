package daemon

import (
	"context"
	"encoding/json"
	"fmt"
	"time"

	"github.com/mitchellh/cli"
	"github.com/pkg/errors"
	"github.com/segmentio/ksuid"
	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/daemon/connector"
	"github.com/vercel/turborepo/cli/internal/daemonclient"
)

func addStatusCmd(root *cobra.Command, config *config.Config, output cli.Ui) {
	var outputJSON bool
	cmd := &cobra.Command{
		Use:           "status",
		Short:         "Reports the status of the turbo daemon",
		SilenceUsage:  true,
		SilenceErrors: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			sessionID := ksuid.New()
			l := &lifecycle{
				sessionID:    sessionID,
				repoRoot:     config.Cwd,
				logger:       config.Logger,
				output:       output,
				turboVersion: config.TurboVersion,
			}
			if err := l.status(outputJSON); err != nil {
				l.logError(err)
				return err
			}
			return nil
		},
	}
	cmd.Flags().BoolVar(&outputJSON, "json", false, "Pass --json to report status in JSON format")
	root.AddCommand(cmd)
}

func (l *lifecycle) status(outputJSON bool) error {
	ctx := context.Background()
	client, err := GetClient(ctx, l.repoRoot, l.logger, l.turboVersion, l.sessionID, ClientOpts{
		// If the daemon is not running, the status is that it's not running.
		// We don't want to start it just to check the status.
		DontStart: true,
	})
	if err != nil {
		return l.reportStatusError(err, outputJSON)
	}
	turboClient := daemonclient.New(client)
	status, err := turboClient.Status(ctx)
	if err != nil {
		return l.reportStatusError(err, outputJSON)
	}
	if outputJSON {
		rendered, err := json.MarshalIndent(status, "", "  ")
		if err != nil {
			return err
		}
		l.output.Output(string(rendered))
	} else {
		uptime := time.Duration(int64(status.UptimeMs * 1000 * 1000))
		l.output.Output(fmt.Sprintf("Daemon log file: %v", status.LogFile))
		l.output.Output(fmt.Sprintf("Daemon uptime: %v", uptime.String()))
		l.output.Output(fmt.Sprintf("Daemon pid file: %v", client.PidPath))
		l.output.Output(fmt.Sprintf("Daemon socket file: %v", client.SockPath))
	}
	return nil
}

func (l *lifecycle) reportStatusError(err error, outputJSON bool) error {
	var msg string
	if errors.Is(err, connector.ErrDaemonNotRunning) {
		msg = "the daemon is not running"
	} else {
		msg = err.Error()
	}
	if outputJSON {
		rendered, err := json.MarshalIndent(map[string]string{
			"error": msg,
		}, "", "  ")
		if err != nil {
			return err
		}
		l.output.Output(string(rendered))
	} else {
		l.output.Output(fmt.Sprintf("Failed to contact daemon: %v", msg))
	}
	return nil
}
