package connector

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"time"

	"github.com/cenkalti/backoff/v4"
	"github.com/hashicorp/go-hclog"
	"github.com/nightlyone/lockfile"
	"github.com/pkg/errors"
	"github.com/vercel/turborepo/cli/internal/turbodprotocol"
	"github.com/vercel/turborepo/cli/internal/turbopath"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/status"
)

var (
	// ErrFailedToStart is returned when the daemon process cannot be started
	ErrFailedToStart     = errors.New("daemon could not be started")
	errVersionMismatch   = errors.New("daemon version does not match client version")
	errConnectionFailure = errors.New("could not connect to daemon")
	// ErrTooManyAttempts is returned when the client fails to connect too many times
	ErrTooManyAttempts = errors.New("reached maximum number of attempts contacting daemon")
	// ErrDaemonNotRunning is returned when the client cannot contact the daemon and has
	// been instructed not to attempt to start a new daemon
	ErrDaemonNotRunning = errors.New("the daemon is not running")
)

// Opts is the set of configurable options for the client connection,
// including some options to be passed through to the daemon process if
// it needs to be started.
type Opts struct {
	ServerTimeout time.Duration
	DontStart     bool // if true, don't attempt to start the daemon
}

// Client represents a connection to the daemon process
type Client struct {
	turbodprotocol.TurbodClient
	*grpc.ClientConn
	SockPath turbopath.AbsoluteSystemPath
	PidPath  turbopath.AbsoluteSystemPath
	LogPath  turbopath.AbsoluteSystemPath
}

// Connector instances are used to create a connection to turbo's daemon process
// The daemon will be started , or killed and restarted, if necessary
type Connector struct {
	Logger       hclog.Logger
	Bin          string
	Opts         Opts
	SockPath     turbopath.AbsoluteSystemPath
	PidPath      turbopath.AbsoluteSystemPath
	LogPath      turbopath.AbsoluteSystemPath
	TurboVersion string
}

// ConnectionError is returned in the error case from connect. It wraps the underlying
// cause and adds a message with the relevant files for the user to check.
type ConnectionError struct {
	SockPath turbopath.AbsoluteSystemPath
	PidPath  turbopath.AbsoluteSystemPath
	LogPath  turbopath.AbsoluteSystemPath
	cause    error
}

func (ce *ConnectionError) Error() string {
	return fmt.Sprintf(`connection to turbo daemon process failed. Please ensure the following:
	- the process identified by the pid in the file at %v is not running, and remove %v
	- check the logs at %v
	- the unix domain socket at %v has been removed
	You can also run without the daemon process by passing --no-daemon`, ce.PidPath, ce.PidPath, ce.LogPath, ce.SockPath)
}

// Unwrap allows a connection error to work with standard library "errors" and compatible packages
func (ce *ConnectionError) Unwrap() error {
	return ce.cause
}

func (c *Connector) wrapConnectionError(err error) error {
	return &ConnectionError{
		SockPath: c.SockPath,
		PidPath:  c.PidPath,
		LogPath:  c.LogPath,
		cause:    err,
	}
}

// lockFile returns a pointer to where a lockfile should be.
// lockfile.New does not perform IO and the only error it produces
// is in the case a non-absolute path was provided. We're guaranteeing an
// turbopath.AbsoluteSystemPath, so an error here is an indication of a bug and
// we should crash.
func (c *Connector) lockFile() lockfile.Lockfile {
	lockFile, err := lockfile.New(c.PidPath.ToString())
	if err != nil {
		panic(err)
	}
	return lockFile
}

func (c *Connector) addr() string {
	// grpc special-cases parsing of unix:<path> urls
	// to avoid url.Parse. This lets us pass through our absolute
	// paths unmodified, even on windows.
	// See code here: https://github.com/grpc/grpc-go/blob/d83070ec0d9043f713b6a63e1963c593b447208c/internal/transport/http_util.go#L392
	return fmt.Sprintf("unix:%v", c.SockPath.ToString())
}

// We defer to the daemon's pid file as the locking mechanism.
// If it doesn't exist, we will attempt to start the daemon.
// If the daemon has a different version, ask it to shut down.
// If the pid file exists but we can't connect, try to kill
// the daemon.
// If we can't cause the daemon to remove the pid file, report
// an error to the user that includes the file location so that
// they can resolve it.
const (
	_maxAttempts       = 3
	_shutdownTimeout   = 1 * time.Second
	_socketPollTimeout = 1 * time.Second
)

// killLiveServer tells a running server to shut down. This method is also responsible
// for closing this client connection.
func (c *Connector) killLiveServer(ctx context.Context, client *Client, serverPid int) error {
	defer func() { _ = client.Close() }()

	_, err := client.Shutdown(ctx, &turbodprotocol.ShutdownRequest{})
	if err != nil {
		c.Logger.Error(fmt.Sprintf("failed to shutdown running daemon. attempting to force it closed: %v", err))
		return c.killDeadServer(serverPid)
	}
	// Wait for the server to gracefully exit
	err = backoff.Retry(func() error {
		lockFile := c.lockFile()
		owner, err := lockFile.GetOwner()
		if os.IsNotExist(err) {
			// If there is no pid more file, we can conclude that the daemon successfully
			// exited and cleaned up after itself.
			return nil
		} else if err != nil {
			// some other error occurred getting the lockfile owner
			return backoff.Permanent(err)
		} else if owner.Pid == serverPid {
			// // We're still waiting for the server to shut down
			return errNeedsRetry
		}
		// if there's no error and the lockfile has a new pid, someone else must've started a new daemon.
		// Consider the old one killed and move on.
		return nil
	}, backoffWithTimeout(_shutdownTimeout))
	if errors.Is(err, errNeedsRetry) {
		c.Logger.Error(fmt.Sprintf("daemon did not exit after %v, attempting to force it closed", _shutdownTimeout.String()))
		return c.killDeadServer(serverPid)
	} else if err != nil {
		return err
	}
	return nil
}

func (c *Connector) killDeadServer(pid int) error {
	// currently the only error that this constructor returns is
	// in the case that you don't provide an absolute path.
	// Given that we require an absolute path as input, this should
	// hopefully never happen.
	lockFile := c.lockFile()
	process, err := lockFile.GetOwner()
	if err == nil {
		// Check that this is the same process that we failed to connect to.
		// Otherwise, connectInternal will loop around again and start with whatever
		// new process has the pid file.
		if process.Pid == pid {
			// we have a process that we need to kill
			// TODO(gsoltis): graceful kill? the process is already not responding to requests,
			// but it could be in the middle of a graceful shutdown. Probably should let it clean
			// itself up, and report an error and defer to a force-kill by the user
			if err := process.Kill(); err != nil {
				return err
			}
		}
		return nil
	} else if errors.Is(err, os.ErrNotExist) {
		// There's no pid file. Someone else killed it. Returning no error will cause the
		// connectInternal to loop around and try the connection again.
		return nil
	}
	return err
}

// Connect attempts to create a connection to a turbo daemon.
// Retries and daemon restarts are built in. If this fails,
// it is unlikely to succeed after an automated retry.
func (c *Connector) Connect(ctx context.Context) (*Client, error) {
	client, err := c.connectInternal(ctx)
	if err != nil {
		return nil, c.wrapConnectionError(err)
	}
	return client, nil
}

func (c *Connector) connectInternal(ctx context.Context) (*Client, error) {
	// for each attempt, we:
	// 1. try to find or start a daemon process, getting its pid
	// 2. wait for the unix domain socket file to appear
	// 3. connect to the unix domain socket. Note that this connection is not validated
	// 4. send a hello message. This validates the connection as a side effect of
	//    negotiating versions, which currently requires exact match.
	// In the event of a live, but incompatible server, we attempt to shut it down and start
	// a new one. In the event of an unresponsive server, we attempt to kill the process
	// identified by the pid file, with the hope that it will clean up after itself.
	// Failures include details about where to find logs, the pid file, and the socket file.
	for i := 0; i < _maxAttempts; i++ {
		serverPid, err := c.getOrStartDaemon()
		if err != nil {
			// If we fail to even start the daemon process, return immediately, we're unlikely
			// to succeed without user intervention
			return nil, err
		}
		if err := c.waitForSocket(); errors.Is(err, ErrFailedToStart) {
			// If we didn't see the socket file, try again. It's possible that
			// the daemon encountered an transitory error
			continue
		} else if err != nil {
			return nil, err
		}
		client, err := c.getClientConn()
		if err != nil {
			return nil, err
		}
		if err := c.sendHello(ctx, client); err == nil {
			// We connected and negotiated a version, we're all set
			return client, nil
		} else if errors.Is(err, errVersionMismatch) {
			// We now know we aren't going to return this client,
			// but killLiveServer still needs it to send the Shutdown request.
			// killLiveServer will close the client when it is done with it.
			if err := c.killLiveServer(ctx, client, serverPid); err != nil {
				return nil, err
			}
		} else if errors.Is(err, errConnectionFailure) {
			// close the client, see if we can kill the stale daemon
			_ = client.Close()
			if err := c.killDeadServer(serverPid); err != nil {
				return nil, err
			}
			// if we successfully killed the dead server, loop around and try again
		} else if err != nil {
			// Some other error occurred, close the client and
			// report the error to the user
			if closeErr := client.Close(); closeErr != nil {
				// In the event that we fail to close the client, bundle that error along also.
				// Keep the original error in the error chain, as it's more likely to be useful
				// or needed for matching on later.
				err = errors.Wrapf(err, "also failed to close client connection: %v", closeErr)
			}
			return nil, err
		}
	}
	return nil, ErrTooManyAttempts
}

// getOrStartDaemon returns the PID of the daemon process on success. It may start
// the daemon if it doesn't find one running.
func (c *Connector) getOrStartDaemon() (int, error) {
	lockFile := c.lockFile()
	if daemonProcess, err := lockFile.GetOwner(); errors.Is(err, lockfile.ErrDeadOwner) {
		// If we've found a pid file but no corresponding process, there's nothing we can do.
		// We defer to the user to clean up the pid file.
		return 0, errors.Wrapf(err, "pid file appears stale. If no daemon is running, please remove it: %v", c.PidPath)
	} else if os.IsNotExist(err) {
		if c.Opts.DontStart {
			return 0, ErrDaemonNotRunning
		}
		// The pid file doesn't exist. Start a daemon
		pid, err := c.startDaemon()
		if err != nil {
			return 0, err
		}
		return pid, nil
	} else {
		return daemonProcess.Pid, nil
	}
}

func (c *Connector) getClientConn() (*Client, error) {
	creds := insecure.NewCredentials()
	conn, err := grpc.Dial(c.addr(), grpc.WithTransportCredentials(creds))
	if err != nil {
		return nil, err
	}
	tc := turbodprotocol.NewTurbodClient(conn)
	return &Client{
		TurbodClient: tc,
		ClientConn:   conn,
		SockPath:     c.SockPath,
		PidPath:      c.PidPath,
		LogPath:      c.LogPath,
	}, nil
}

func (c *Connector) sendHello(ctx context.Context, client turbodprotocol.TurbodClient) error {
	_, err := client.Hello(ctx, &turbodprotocol.HelloRequest{
		Version: c.TurboVersion,
		// TODO: add session id
	})
	status := status.Convert(err)
	switch status.Code() {
	case codes.OK:
		return nil
	case codes.FailedPrecondition:
		return errVersionMismatch
	case codes.Unavailable:
		return errConnectionFailure
	default:
		return err
	}
}

var errNeedsRetry = errors.New("retry the operation")

// backoffWithTimeout returns an exponential backoff, starting at 2ms and doubling until
// the specific timeout has elapsed. Note that backoff instances are stateful, so we need
// a new one each time we do a Retry.
func backoffWithTimeout(timeout time.Duration) *backoff.ExponentialBackOff {
	return &backoff.ExponentialBackOff{
		Multiplier:      2,
		InitialInterval: 2 * time.Millisecond,
		MaxElapsedTime:  timeout,
		Clock:           backoff.SystemClock,
		Stop:            backoff.Stop,
	}
}

// waitForSocket waits for the unix domain socket to appear
func (c *Connector) waitForSocket() error {
	// Note that we don't care if this is our daemon
	// or not. We started a process, but someone else could beat
	// use to listening. That's fine, we'll check the version
	// later.
	err := backoff.Retry(func() error {
		if !c.SockPath.FileExists() {
			return errNeedsRetry
		}
		return nil
	}, backoffWithTimeout(_socketPollTimeout))
	if errors.Is(err, errNeedsRetry) {
		return ErrFailedToStart
	} else if err != nil {
		return err
	}
	return nil
}

// startDaemon starts the daemon and returns the pid for the new process
func (c *Connector) startDaemon() (int, error) {
	args := []string{"daemon"}
	if c.Opts.ServerTimeout != 0 {
		args = append(args, fmt.Sprintf("--idle-time=%v", c.Opts.ServerTimeout.String()))
	}
	c.Logger.Debug(fmt.Sprintf("starting turbod binary %v", c.Bin))
	cmd := exec.Command(c.Bin, args...)
	// For the daemon to have its own process group id so that any attempts
	// to kill it and its process tree don't kill this client.
	cmd.SysProcAttr = getSysProcAttrs()
	err := cmd.Start()
	if err != nil {
		return 0, err
	}
	return cmd.Process.Pid, nil
}
