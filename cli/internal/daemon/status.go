package daemon

import (
	"context"
	"encoding/json"
	"fmt"
	"time"

	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
	"github.com/pkg/errors"
	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/daemon/connector"
	"github.com/vercel/turborepo/cli/internal/daemonclient"
	"github.com/vercel/turborepo/cli/internal/fs"
)

func addStatusCmd(root *cobra.Command, config *config.Config, output cli.Ui) {
	var outputJSON bool
	cmd := &cobra.Command{
		Use:           "status",
		Short:         "Reports the status of the turbo daemon",
		SilenceUsage:  true,
		SilenceErrors: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			s := &status{
				logger:       config.Logger,
				output:       output,
				turboVersion: config.TurboVersion,
				repoRoot:     config.Cwd,
				outputJSON:   outputJSON,
			}
			return s.status()
		},
	}
	cmd.Flags().BoolVar(&outputJSON, "json", false, "Pass --json to report status in JSON format")
	root.AddCommand(cmd)
}

type status struct {
	logger       hclog.Logger
	output       cli.Ui
	turboVersion string
	repoRoot     fs.AbsolutePath
	outputJSON   bool
}

func (s *status) status() error {
	ctx := context.Background()
	client, err := GetClient(ctx, s.repoRoot, s.logger, s.turboVersion, ClientOpts{
		DontStart: true,
	})
	if err != nil {
		return s.reportError(err)
	}
	turboClient := daemonclient.New(ctx, client)
	status, err := turboClient.Status()
	if err != nil {
		return s.reportError(err)
	}
	if s.outputJSON {
		rendered, err := json.MarshalIndent(map[string]interface{}{
			"logFile":  status.LogFile,
			"uptimeMs": status.UptimeMsec,
		}, "", "  ")
		if err != nil {
			return err
		}
		s.output.Output(string(rendered))
	} else {
		uptime := time.Duration(int64(status.UptimeMsec * 1000))
		s.output.Output(fmt.Sprintf("Daemon log file: %v", status.LogFile))
		s.output.Output(fmt.Sprintf("Daemon uptime: %v", uptime.String()))
	}
	return nil
}

func (s *status) reportError(err error) error {
	var msg string
	if errors.Is(err, connector.ErrDaemonNotRunning) {
		msg = "the daemon is not running"
	} else {
		msg = err.Error()
	}
	if s.outputJSON {
		rendered, err := json.MarshalIndent(map[string]string{
			"error": msg,
		}, "", "  ")
		if err != nil {
			return err
		}
		s.output.Output(string(rendered))
	} else {
		s.output.Output(fmt.Sprintf("Failed to contact daemon: %v", msg))
	}
	return nil
}
