package daemon

import (
	"context"
	"fmt"
	"github.com/vercel/turbo/cli/internal/turbostate"

	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/cmdutil"
	"github.com/vercel/turbo/cli/internal/daemon/connector"
	"github.com/vercel/turbo/cli/internal/turbodprotocol"
)

// RunLifecycle executes the lifecycle commands `start`, `stop`, `restart`.
func RunLifecycle(ctx context.Context, helper *cmdutil.Helper, args *turbostate.ParsedArgsFromRust) error {
	base, err := helper.GetCmdBase(args)
	if err != nil {
		return err
	}
	l := &lifecycle{
		base,
	}

	if args.Command.Daemon.Command == "Restart" {
		if err := l.ensureStopped(ctx); err != nil {
			l.logError(err)
			return err
		}
		if err := l.ensureStarted(ctx); err != nil {
			l.logError(err)
			return err
		}
	} else if args.Command.Daemon.Command == "Start" {
		if err := l.ensureStarted(ctx); err != nil {
			l.logError(err)
			return err
		}
	} else if args.Command.Daemon.Command == "Stop" {
		if err := l.ensureStopped(ctx); err != nil {
			l.logError(err)
			return err
		}
	}

	return nil
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
