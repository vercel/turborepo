package daemon

import (
	"context"
	"fmt"

	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
	"github.com/pkg/errors"
	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/daemon/connector"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/server"
)

func addStartCmd(root *cobra.Command, config *config.Config, output cli.Ui) {
	cmd := &cobra.Command{
		Use:           "start",
		Short:         "Ensures that the turbo daemon is running",
		SilenceUsage:  true,
		SilenceErrors: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			l := &lifecycle{
				repoRoot:     config.Cwd,
				logger:       config.Logger,
				output:       output,
				turboVersion: config.TurboVersion,
			}
			return l.start()
		},
	}
	root.AddCommand(cmd)
}

func addStopCmd(root *cobra.Command, config *config.Config, output cli.Ui) {
	cmd := &cobra.Command{
		Use:           "stop",
		Short:         "Ensures that the turbo daemon is running",
		SilenceUsage:  true,
		SilenceErrors: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			l := &lifecycle{
				repoRoot:     config.Cwd,
				logger:       config.Logger,
				output:       output,
				turboVersion: config.TurboVersion,
			}
			return l.stop()
		},
	}
	root.AddCommand(cmd)
}

func addRestartCmd(root *cobra.Command, config *config.Config, output cli.Ui) {
	cmd := &cobra.Command{
		Use:           "restart",
		Short:         "Ensures that the turbo daemon is running",
		SilenceUsage:  true,
		SilenceErrors: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			l := &lifecycle{
				repoRoot:     config.Cwd,
				logger:       config.Logger,
				output:       output,
				turboVersion: config.TurboVersion,
			}
			if err := l.stop(); err != nil {
				return err
			}
			if err := l.start(); err != nil {
				return err
			}
			return nil
		},
	}
	root.AddCommand(cmd)
}

type lifecycle struct {
	repoRoot     fs.AbsolutePath
	logger       hclog.Logger
	output       cli.Ui
	turboVersion string
}

func (l *lifecycle) start() error {
	ctx := context.Background()
	client, err := GetClient(ctx, l.repoRoot, l.logger, l.turboVersion, ClientOpts{})
	if err != nil {
		l.output.Error(fmt.Sprintf("Failed to start turbo daemon: %v", err))
		return err
	}
	// We don't really care if we fail to close the client, we're about to exit
	_ = client.Close()
	l.output.Output("turbo daemon is running")
	return nil
}

func (l *lifecycle) stop() error {
	ctx := context.Background()
	client, err := GetClient(ctx, l.repoRoot, l.logger, l.turboVersion, ClientOpts{
		DontStart: true,
	})
	if err != nil {
		if errors.Is(err, connector.ErrDaemonNotRunning) {
			l.output.Output("turbo daemon is not running")
			return nil
		}
		l.output.Error(fmt.Sprintf("Failed to contact turbo daemon: %v", err))
		return err
	}
	defer func() { _ = client.Close() }()
	_, err = client.Shutdown(ctx, &server.ShutdownRequest{})
	if err != nil {
		l.output.Error(fmt.Sprintf("Failed to shut down turbo daemon: %v", err))
		return err
	}
	l.output.Output("Successfully requested that turbo daemon shut down")
	return nil
}
