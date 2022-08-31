package daemon

import (
	"context"

	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
	"github.com/pkg/errors"
	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/daemon/connector"
	"github.com/vercel/turborepo/cli/internal/turbodprotocol"
	"github.com/vercel/turborepo/cli/internal/turbopath"
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
			if err := l.ensureStarted(); err != nil {
				l.logError(err)
				return err
			}
			return nil
		},
	}
	root.AddCommand(cmd)
}

func addStopCmd(root *cobra.Command, config *config.Config, output cli.Ui) {
	cmd := &cobra.Command{
		Use:           "stop",
		Short:         "Stop the turbo daemon",
		SilenceUsage:  true,
		SilenceErrors: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			l := &lifecycle{
				repoRoot:     config.Cwd,
				logger:       config.Logger,
				output:       output,
				turboVersion: config.TurboVersion,
			}
			if err := l.ensureStopped(); err != nil {
				l.logError(err)
				return err
			}
			return nil
		},
	}
	root.AddCommand(cmd)
}

func addRestartCmd(root *cobra.Command, config *config.Config, output cli.Ui) {
	cmd := &cobra.Command{
		Use:           "restart",
		Short:         "Restart the turbo daemon",
		SilenceUsage:  true,
		SilenceErrors: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			l := &lifecycle{
				repoRoot:     config.Cwd,
				logger:       config.Logger,
				output:       output,
				turboVersion: config.TurboVersion,
			}
			if err := l.ensureStopped(); err != nil {
				l.logError(err)
				return err
			}
			if err := l.ensureStarted(); err != nil {
				l.logError(err)
				return err
			}
			return nil
		},
	}
	root.AddCommand(cmd)
}

type lifecycle struct {
	repoRoot     turbopath.AbsolutePath
	logger       hclog.Logger
	output       cli.Ui
	turboVersion string
}

// logError logs an error and outputs it to the UI.
func (l *lifecycle) logError(err error) {
	l.logger.Error("error", err)
	l.output.Error(err.Error())
}

func (l *lifecycle) ensureStarted() error {
	ctx := context.Background()
	client, err := GetClient(ctx, l.repoRoot, l.logger, l.turboVersion, ClientOpts{})
	if err != nil {
		return err
	}
	// We don't really care if we fail to close the client, we're about to exit
	_ = client.Close()
	l.output.Output("turbo daemon is running")
	return nil
}

func (l *lifecycle) ensureStopped() error {
	ctx := context.Background()
	client, err := GetClient(ctx, l.repoRoot, l.logger, l.turboVersion, ClientOpts{
		// If the daemon is not running, don't start it, since we're trying to stop it
		DontStart: true,
	})
	if err != nil {
		if errors.Is(err, connector.ErrDaemonNotRunning) {
			l.output.Output("turbo daemon is not running")
			return nil
		}
		return err
	}
	defer func() { _ = client.Close() }()
	_, err = client.Shutdown(ctx, &turbodprotocol.ShutdownRequest{})
	if err != nil {
		return err
	}
	l.output.Output("Successfully requested that turbo daemon shut down")
	return nil
}
