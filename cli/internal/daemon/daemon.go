package daemon

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io"
	"net"
	"os"
	"path/filepath"
	"strings"
	"time"

	grpc_recovery "github.com/grpc-ecosystem/go-grpc-middleware/recovery"
	"github.com/hashicorp/go-hclog"
	"github.com/nightlyone/lockfile"
	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/cmdutil"
	"github.com/vercel/turbo/cli/internal/daemon/connector"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/server"
	"github.com/vercel/turbo/cli/internal/signals"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/turbostate"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

type daemon struct {
	logger     hclog.Logger
	repoRoot   turbopath.AbsoluteSystemPath
	timeout    time.Duration
	reqCh      chan struct{}
	timedOutCh chan struct{}
}

func getRepoHash(repoRoot turbopath.AbsoluteSystemPath) string {
	pathHash := sha256.Sum256([]byte(repoRoot.ToString()))
	// We grab a substring of the hash because there is a 108-character limit on the length
	// of a filepath for unix domain socket.
	return hex.EncodeToString(pathHash[:])[:16]
}

func getDaemonFileRoot(repoRoot turbopath.AbsoluteSystemPath) turbopath.AbsoluteSystemPath {
	tempDir := fs.TempDir("turbod")
	hexHash := getRepoHash(repoRoot)
	return tempDir.UntypedJoin(hexHash)
}

func getLogFilePath(repoRoot turbopath.AbsoluteSystemPath) (turbopath.AbsoluteSystemPath, error) {
	hexHash := getRepoHash(repoRoot)
	base := repoRoot.Base()
	logFilename := fmt.Sprintf("%v-%v.log", hexHash, base)

	logsDir := fs.GetTurboDataDir().UntypedJoin("logs")
	return logsDir.UntypedJoin(logFilename), nil
}

func getUnixSocket(repoRoot turbopath.AbsoluteSystemPath) turbopath.AbsoluteSystemPath {
	root := getDaemonFileRoot(repoRoot)
	return root.UntypedJoin("turbod.sock")
}

func getPidFile(repoRoot turbopath.AbsoluteSystemPath) turbopath.AbsoluteSystemPath {
	root := getDaemonFileRoot(repoRoot)
	return root.UntypedJoin("turbod.pid")
}

// logError logs an error and outputs it to the UI.
func (d *daemon) logError(err error) {
	d.logger.Error(fmt.Sprintf("error %v", err))
}

// we're only appending, and we're creating the file if it doesn't exist.
// we do not need to read the log file.
var _logFileFlags = os.O_WRONLY | os.O_APPEND | os.O_CREATE

// ExecuteDaemon executes the root daemon command
func ExecuteDaemon(ctx context.Context, helper *cmdutil.Helper, signalWatcher *signals.Watcher, executionState *turbostate.ExecutionState) error {
	base, err := helper.GetCmdBase(executionState)
	if err != nil {
		return err
	}
	if executionState.CLIArgs.TestRun {
		base.UI.Info("Daemon test run successful")
		return nil
	}

	idleTimeout := 4 * time.Hour
	if executionState.CLIArgs.Command.Daemon.IdleTimeout != "" {
		idleTimeout, err = time.ParseDuration(executionState.CLIArgs.Command.Daemon.IdleTimeout)
		if err != nil {
			return err
		}
	}

	logFilePath, err := getLogFilePath(base.RepoRoot)
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
		Level:  hclog.Info,
		Color:  hclog.ColorOff,
		Name:   "turbod",
	})

	d := &daemon{
		logger:     logger,
		repoRoot:   base.RepoRoot,
		timeout:    idleTimeout,
		reqCh:      make(chan struct{}),
		timedOutCh: make(chan struct{}),
	}
	serverName := getRepoHash(base.RepoRoot)
	turboServer, err := server.New(serverName, d.logger.Named("rpc server"), base.RepoRoot, base.TurboVersion, logFilePath)
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
}

var errInactivityTimeout = errors.New("turbod shut down from inactivity")

// tryAcquirePidfileLock attempts to ensure that only one daemon is running from the given pid file path
// at a time. If this process fails to write its PID to the lockfile, it must exit.
func tryAcquirePidfileLock(pidPath turbopath.AbsoluteSystemPath) (lockfile.Lockfile, error) {
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
func GetClient(ctx context.Context, repoRoot turbopath.AbsoluteSystemPath, logger hclog.Logger, turboVersion string, opts ClientOpts) (*Client, error) {
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
	// The Go binary can no longer be called directly, so we need to route back to the rust wrapper
	if strings.HasSuffix(bin, "go-turbo") {
		bin = filepath.Join(filepath.Dir(bin), "turbo")
	} else if strings.HasSuffix(bin, "go-turbo.exe") {
		bin = filepath.Join(filepath.Dir(bin), "turbo.exe")
	}
	c := &connector.Connector{
		Logger:       logger.Named("TurbodClient"),
		Bin:          bin,
		Opts:         opts,
		SockPath:     sockPath,
		PidPath:      pidPath,
		LogPath:      logPath,
		TurboVersion: turboVersion,
	}
	return c.Connect(ctx)
}
