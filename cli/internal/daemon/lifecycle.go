package daemon

import (
	"context"
	"fmt"

	"github.com/pkg/errors"
	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/cmdutil"
	"github.com/vercel/turborepo/cli/internal/daemon/connector"
	"github.com/vercel/turborepo/cli/internal/turbodprotocol"
)

func addStartCmd(root *cobra.Command, helper *cmdutil.Helper) {
	cmd := &cobra.Command{
		Use:           "start",
		Short:         "Ensures that the turbo daemon is running",
		SilenceUsage:  true,
		SilenceErrors: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			base, err := helper.GetCmdBase(cmd.Flags())
			if err != nil {
				return err
			}
			l := &lifecycle{
				base,
			}
			if err := l.ensureStarted(cmd.Context()); err != nil {
				l.logError(err)
				return err
			}
			return nil
		},
	}
	root.AddCommand(cmd)
}

func addStopCmd(root *cobra.Command, helper *cmdutil.Helper) {
	cmd := &cobra.Command{
		Use:           "stop",
		Short:         "Stop the turbo daemon",
		SilenceUsage:  true,
		SilenceErrors: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			base, err := helper.GetCmdBase(cmd.Flags())
			if err != nil {
				return err
			}
			l := &lifecycle{
				base,
			}
			if err := l.ensureStopped(cmd.Context()); err != nil {
				l.logError(err)
				return err
			}
			return nil
		},
	}
	root.AddCommand(cmd)
}

func addRestartCmd(root *cobra.Command, helper *cmdutil.Helper) {
	cmd := &cobra.Command{
		Use:           "restart",
		Short:         "Restart the turbo daemon",
		SilenceUsage:  true,
		SilenceErrors: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			base, err := helper.GetCmdBase(cmd.Flags())
			if err != nil {
				return err
			}
			l := &lifecycle{
				base,
			}
			if err := l.ensureStopped(cmd.Context()); err != nil {
				l.logError(err)
				return err
			}
			if err := l.ensureStarted(cmd.Context()); err != nil {
				l.logError(err)
				return err
			}
			return nil
		},
	}
	root.AddCommand(cmd)
}

type lifecycle struct {
	base *cmdutil.CmdBase
}

// logError logs an error and outputs it to the UI.
func (l *lifecycle) logError(err error) {
	l.base.Logger.Error(fmt.Sprintf("error: %v", err))
	l.base.UI.Error(err.Error())
}

func (l *lifecycle) ensureStarted(ctx context.Context) error {
	client, err := GetClient(ctx, l.base.RepoRoot, l.base.Logger, l.base.TurboVersion, ClientOpts{})
	if err != nil {
		return err
	}
	// We don't really care if we fail to close the client, we're about to exit
	_ = client.Close()
	l.base.UI.Output("turbo daemon is running")
	return nil
}

func (l *lifecycle) ensureStopped(ctx context.Context) error {
	client, err := GetClient(ctx, l.base.RepoRoot, l.base.Logger, l.base.TurboVersion, ClientOpts{
		// If the daemon is not running, don't start it, since we're trying to stop it
		DontStart: true,
	})
	if err != nil {
		if errors.Is(err, connector.ErrDaemonNotRunning) {
			l.base.UI.Output("turbo daemon is not running")
			return nil
		}
		return err
	}
	defer func() { _ = client.Close() }()
	_, err = client.Shutdown(ctx, &turbodprotocol.ShutdownRequest{})
	if err != nil {
		return err
	}
	l.base.UI.Output("Successfully requested that turbo daemon shut down")
	return nil
}
