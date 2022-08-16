package daemon

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io"
	"net"
	"os"
	"time"

	grpc_recovery "github.com/grpc-ecosystem/go-grpc-middleware/recovery"
	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
	"github.com/nightlyone/lockfile"
	"github.com/pkg/errors"
	"github.com/segmentio/ksuid"
	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/daemon/connector"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/server"
	"github.com/vercel/turborepo/cli/internal/signals"
	"github.com/vercel/turborepo/cli/internal/util"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

// Command is the wrapper around the daemon command until we port fully to cobra
type Command struct {
	Config        *config.Config
	UI            cli.Ui
	SignalWatcher *signals.Watcher
}

// Run runs the daemon command
func (c *Command) Run(args []string) int {
	cmd := getCmd(c.Config, c.UI, c.SignalWatcher)
	cmd.SetArgs(args)
	err := cmd.Execute()
	if err != nil {
		return 1
	}
	return 0
}

// Help returns information about the `daemon` command
func (c *Command) Help() string {
	cmd := getCmd(c.Config, c.UI, c.SignalWatcher)
	return util.HelpForCobraCmd(cmd)
}

// Synopsis of daemon command
func (c *Command) Synopsis() string {
	cmd := getCmd(c.Config, c.UI, c.SignalWatcher)
	return cmd.Short
}

type daemon struct {
	logger     hclog.Logger
	repoRoot   fs.AbsolutePath
	timeout    time.Duration
	reqCh      chan struct{}
	timedOutCh chan struct{}
}

func getRepoHash(repoRoot fs.AbsolutePath) string {
	pathHash := sha256.Sum256([]byte(repoRoot.ToString()))
	// We grab a substring of the hash because there is a 108-character limit on the length
	// of a filepath for unix domain socket.
	return hex.EncodeToString(pathHash[:])[:16]
}

func getDaemonFileRoot(repoRoot fs.AbsolutePath) fs.AbsolutePath {
	tempDir := fs.TempDir("turbod")
	hexHash := getRepoHash(repoRoot)
	return tempDir.Join(hexHash)
}

func getLogFilePath(repoRoot fs.AbsolutePath) (fs.AbsolutePath, error) {
	hexHash := getRepoHash(repoRoot)
	base := repoRoot.Base()
	logFilename := fmt.Sprintf("%v-%v.log", hexHash, base)

	logsDir := fs.GetTurboDataDir().Join("logs")
	return logsDir.Join(logFilename), nil
}

func getUnixSocket(repoRoot fs.AbsolutePath) fs.AbsolutePath {
	root := getDaemonFileRoot(repoRoot)
	return root.Join("turbod.sock")
}

func getPidFile(repoRoot fs.AbsolutePath) fs.AbsolutePath {
	root := getDaemonFileRoot(repoRoot)
	return root.Join("turbod.pid")
}

// logError logs an error and outputs it to the UI.
func (d *daemon) logError(err error) {
	d.logger.Error("error", err)
}

// we're only appending, and we're creating the file if it doesn't exist.
// we do not need to read the log file.
var _logFileFlags = os.O_WRONLY | os.O_APPEND | os.O_CREATE

func getCmd(config *config.Config, output cli.Ui, signalWatcher *signals.Watcher) *cobra.Command {
	var idleTimeout time.Duration
	cmd := &cobra.Command{
		Use:           "turbo daemon",
		Short:         "Runs turbod",
		SilenceUsage:  true,
		SilenceErrors: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			logFilePath, err := getLogFilePath(config.Cwd)
			if err != nil {
				return err
			}
			if err := logFilePath.EnsureDir(); err != nil {
				return err
			}
			logFile, err := logFilePath.OpenFile(_logFileFlags, 0644)
			if err != nil {
				return err
			}
			defer func() { _ = logFile.Close() }()
			logger := hclog.New(&hclog.LoggerOptions{
				Output: io.MultiWriter(logFile, os.Stdout),
				Level:  hclog.Debug,
				Color:  hclog.ColorOff,
				Name:   "turbod",
			})
			ctx := cmd.Context()
			d := &daemon{
				logger:     logger,
				repoRoot:   config.Cwd,
				timeout:    idleTimeout,
				reqCh:      make(chan struct{}),
				timedOutCh: make(chan struct{}),
			}
			serverName := getRepoHash(config.Cwd)
			turboServer, err := server.New(serverName, d.logger.Named("rpc server"), config.Cwd, config.TurboVersion, logFilePath)
			if err != nil {
				d.logError(err)
				return err
			}
			defer func() { _ = turboServer.Close() }()
			err = d.runTurboServer(ctx, turboServer, signalWatcher)
			if err != nil {
				d.logError(err)
				return err
			}
			return nil
		},
	}
	cmd.Flags().DurationVar(&idleTimeout, "idle-time", 4*time.Hour, "Set the idle timeout for turbod")
	addDaemonSubcommands(cmd, config, output)
	return cmd
}

func addDaemonSubcommands(cmd *cobra.Command, config *config.Config, output cli.Ui) {
	addStatusCmd(cmd, config, output)
	addStartCmd(cmd, config, output)
	addStopCmd(cmd, config, output)
	addRestartCmd(cmd, config, output)
}

var errInactivityTimeout = errors.New("turbod shut down from inactivity")

// tryAcquirePidfileLock attempts to ensure that only one daemon is running from the given pid file path
// at a time. If this process fails to write its PID to the lockfile, it must exit.
func tryAcquirePidfileLock(pidPath fs.AbsolutePath) (lockfile.Lockfile, error) {
	if err := pidPath.EnsureDir(); err != nil {
		return "", err
	}
	lockFile, err := lockfile.New(pidPath.ToString())
	if err != nil {
		// lockfile.New should only return an error if it wasn't given an absolute path.
		// We are attempting to use the type system to enforce that we are passing an
		// absolute path. An error here likely means a bug, and we should crash.
		panic(err)
	}
	if err := lockFile.TryLock(); err != nil {
		return "", err
	}
	return lockFile, nil
}

type rpcServer interface {
	Register(grpcServer server.GRPCServer)
}

func (d *daemon) runTurboServer(parentContext context.Context, rpcServer rpcServer, signalWatcher *signals.Watcher) error {
	ctx, cancel := context.WithCancel(parentContext)
	defer cancel()
	pidPath := getPidFile(d.repoRoot)
	lock, err := tryAcquirePidfileLock(pidPath)
	if err != nil {
		return errors.Wrapf(err, "failed to lock the pid file at %v. Is another turbo daemon running?", lock)
	}
	// When we're done serving, clean up the pid file.
	// Also, if *this* goroutine panics, make sure we unlock the pid file.
	defer func() {
		if err := lock.Unlock(); err != nil {
			d.logger.Error(errors.Wrapf(err, "failed unlocking pid file at %v", lock).Error())
		}
	}()
	// This handler runs in request goroutines. If a request causes a panic,
	// this handler will get called after a call to recover(), meaning we are
	// no longer panicking. We return a server error and cancel our context,
	// which triggers a shutdown of the server.
	panicHandler := func(thePanic interface{}) error {
		cancel()
		d.logger.Error(fmt.Sprintf("Caught panic %v", thePanic))
		return status.Error(codes.Internal, "server panicked")
	}

	// If we have the lock, assume that we are the owners of the socket file,
	// whether it already exists or not. That means we are free to remove it.
	sockPath := getUnixSocket(d.repoRoot)
	if err := sockPath.Remove(); err != nil && !errors.Is(err, os.ErrNotExist) {
		return err
	}
	d.logger.Debug(fmt.Sprintf("Using socket path %v (%v)\n", sockPath, len(sockPath)))
	lis, err := net.Listen("unix", sockPath.ToString())
	if err != nil {
		return err
	}
	// We don't need to explicitly close 'lis', the grpc server will handle that
	s := grpc.NewServer(
		grpc.ChainUnaryInterceptor(
			d.onRequest,
			grpc_recovery.UnaryServerInterceptor(grpc_recovery.WithRecoveryHandler(panicHandler)),
		),
	)
	go d.timeoutLoop(ctx)

	rpcServer.Register(s)
	errCh := make(chan error)
	go func(errCh chan<- error) {
		if err := s.Serve(lis); err != nil {
			errCh <- err
		}
		close(errCh)
	}(errCh)

	// Note that we aren't deferring s.GracefulStop here because we also need
	// to drain the error channel, which isn't guaranteed to happen until
	// the server has stopped. That in turn may depend on GracefulStop being
	// called.
	// Future work could restructure this to make that simpler.
	var exitErr error
	select {
	case err, ok := <-errCh:
		// The server exited
		if ok {
			exitErr = err
		}
	case <-d.timedOutCh:
		// This is the inactivity timeout case
		exitErr = errInactivityTimeout
		s.GracefulStop()
	case <-ctx.Done():
		// If a request handler panics, it will cancel this context
		s.GracefulStop()
	case <-signalWatcher.Done():
		// This is fired if caught a signal
		s.GracefulStop()
	}
	// Wait for the server to exit, if it hasn't already.
	// When it does, this channel will close. We don't
	// care about the error in this scenario because we've
	// either requested a close via cancelling the context,
	// an inactivity timeout, or caught a signal.
	for range errCh {
	}
	return exitErr
}

func (d *daemon) onRequest(ctx context.Context, req interface{}, info *grpc.UnaryServerInfo, handler grpc.UnaryHandler) (resp interface{}, err error) {
	d.reqCh <- struct{}{}
	return handler(ctx, req)
}

func (d *daemon) timeoutLoop(ctx context.Context) {
	timeoutCh := time.After(d.timeout)
outer:
	for {
		select {
		case <-d.reqCh:
			timeoutCh = time.After(d.timeout)
		case <-timeoutCh:
			close(d.timedOutCh)
			break outer
		case <-ctx.Done():
			break outer
		}
	}
}

// ClientOpts re-exports connector.Ops to encapsulate the connector package
type ClientOpts = connector.Opts

// Client re-exports connector.Client to encapsulate the connector package
type Client = connector.Client

// GetClient returns a client that can be used to interact with the daemon
func GetClient(ctx context.Context, repoRoot fs.AbsolutePath, logger hclog.Logger, turboVersion string, sessionID ksuid.KSUID, opts ClientOpts) (*Client, error) {
	sockPath := getUnixSocket(repoRoot)
	pidPath := getPidFile(repoRoot)
	logPath, err := getLogFilePath(repoRoot)
	if err != nil {
		return nil, err
	}
	bin, err := os.Executable()
	if err != nil {
		return nil, err
	}
	c := &connector.Connector{
		SessionID:    sessionID,
		Logger:       logger.Named("TurbodClient"),
		Bin:          bin,
		Opts:         opts,
		SockPath:     sockPath,
		PidPath:      pidPath,
		LogPath:      logPath,
		TurboVersion: turboVersion,
	}
	client, err := c.Connect(ctx)
	if err != nil {
		return nil, err
	}
	return client, nil
}
