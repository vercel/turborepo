package daemon

import (
	"context"
	"encoding/json"
	"fmt"
	"github.com/vercel/turbo/cli/internal/config"
	"time"

	"github.com/pkg/errors"
	"github.com/spf13/cobra"
	"github.com/vercel/turbo/cli/internal/cmdutil"
	"github.com/vercel/turbo/cli/internal/daemon/connector"
	"github.com/vercel/turbo/cli/internal/daemonclient"
)

func addStatusCmd(root *cobra.Command, helper *cmdutil.Helper) {
	var outputJSON bool
	cmd := &cobra.Command{
		Use:           "status",
		Short:         "Reports the status of the turbo daemon",
		SilenceUsage:  true,
		SilenceErrors: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			flags := config.FlagSet{FlagSet: cmd.Flags()}
			base, err := helper.GetCmdBase(flags)
			if err != nil {
				return err
			}
			l := &lifecycle{
				base,
			}
			if err := l.status(cmd.Context(), outputJSON); err != nil {
				l.logError(err)
				return err
			}
			return nil
		},
	}
	cmd.Flags().BoolVar(&outputJSON, "json", false, "Pass --json to report status in JSON format")
	root.AddCommand(cmd)
}

func (l *lifecycle) status(ctx context.Context, outputJSON bool) error {
	client, err := GetClient(ctx, l.base.RepoRoot, l.base.Logger, l.base.TurboVersion, ClientOpts{
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
		l.base.UI.Output(string(rendered))
	} else {
		uptime := time.Duration(int64(status.UptimeMs * 1000 * 1000))
		l.base.UI.Output(fmt.Sprintf("Daemon log file: %v", status.LogFile))
		l.base.UI.Output(fmt.Sprintf("Daemon uptime: %v", uptime.String()))
		l.base.UI.Output(fmt.Sprintf("Daemon pid file: %v", client.PidPath))
		l.base.UI.Output(fmt.Sprintf("Daemon socket file: %v", client.SockPath))
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
		l.base.UI.Output(string(rendered))
	} else {
		l.base.UI.Output(fmt.Sprintf("Failed to contact daemon: %v", msg))
	}
	return nil
}
