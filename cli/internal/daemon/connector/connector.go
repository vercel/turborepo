package connector

import (
	"context"
	"fmt"
	"io"
	"os"
	"os/exec"
	"time"

	"github.com/hashicorp/go-hclog"
	"github.com/nightlyone/lockfile"
	"github.com/pkg/errors"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/server"
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
)

// Opts is the set of configurable options for the client connection,
// including some options to be passed through to the daemon process if
// it needs to be started.
type Opts struct {
	ServerTimeout time.Duration
}

// Client represents a connection to the daemon process
type Client interface {
	server.TurboClient
	io.Closer
}

type clientAndConn struct {
	server.TurboClient
	*grpc.ClientConn
}

// Connector instances are used to create a connection to turbo's daemon process
// The daemon will be started , or killed and restarted, if necessary
type Connector struct {
	Logger       hclog.Logger
	Bin          string
	Opts         Opts
	SockPath     fs.AbsolutePath
	PidPath      fs.AbsolutePath
	Ctx          context.Context
	TurboVersion string
}

func (c *Connector) wrapConnectionError(err error) error {
	return errors.Wrapf(err, `connection to turbo daemon process failed. Please ensure the following:
 - the unix domain socket at %v has been removed
 - the process identified by the pid at %v is not running, and remove %v
 You can also run without the daemon process by passing --no-daemon`, c.SockPath, c.PidPath, c.PidPath)
}

func (c *Connector) addr() string {
	return fmt.Sprintf("unix://%v", c.SockPath.ToString())
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
	_maxAttempts          = 3
	_shutdownDeadline     = 1 * time.Second
	_shutdownPollInterval = 50 * time.Millisecond
	// keep a tighter loop for this one, it's in the normal startup path
	_socketPollInterval = 10 * time.Millisecond
	_socketPollTimeout  = 1 * time.Second
)

// killLiveServer tells a running server to shut down. This method is also responsible
// for closing this connection
func (c *Connector) killLiveServer(client *clientAndConn, serverPid int) error {
	defer func() { _ = client.Close() }()

	_, err := client.Shutdown(c.Ctx, &server.ShutdownRequest{})
	if err != nil {
		c.Logger.Error(fmt.Sprintf("failed to shutdown running daemon. attempting to force it closed: %v", err))
		return c.killDeadServer(serverPid)
	}
	// Wait for the server to gracefully exit
	deadline := time.After(_shutdownDeadline)
	for {
		lockFile, err := lockfile.New(c.PidPath.ToString())
		if err != nil {
			return err
		}
		owner, err := lockFile.GetOwner()
		if os.IsNotExist(err) {
			return nil
		} else if err != nil {
			return err
		} else if owner.Pid == serverPid {
			// We're still waiting for the server to shut down
			select {
			case <-deadline:
				c.Logger.Error(fmt.Sprintf("daemon did not exit after %v, attempting to force it closed", _shutdownDeadline.String()))
				return c.killDeadServer(serverPid)
			case <-time.After(_shutdownPollInterval):
				// loop around and check again
			}
		}
	}
}

func (c *Connector) killDeadServer(pid int) error {
	// currently the only error that this constructor returns is
	// in the case that you don't provide an absolute path.
	// Given that we require an absolute path as input, this should
	// hopefully never happen.
	lockFile, err := lockfile.New(c.PidPath.ToString())
	if err != nil {
		return err
	}
	process, err := lockFile.GetOwner()
	if err == nil {
		// Check that this is the same process that we failed to connect to.
		// Otherwise, loop around again and start with whatever new process has
		// the pid file.
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
		// There's no pid file. Someone else killed it. Loop around and try again
		return nil
	} else if errors.Is(err, lockfile.ErrDeadOwner) {
		// The daemon crashed. report an error to the user
		return err
	}
	return err
}

// Connect attempts to create a connection to a turbo daemon.
// Retries and daemon restarts are built in. If this fails,
// it is unlikely to succeed after an automated retry.
func (c *Connector) Connect() (Client, error) {
	client, err := c.connectInternal()
	if err != nil {
		return nil, c.wrapConnectionError(err)
	}
	return client, nil
}

func (c *Connector) connectInternal() (*clientAndConn, error) {
	for i := 0; i < _maxAttempts; i++ {
		lockFile, err := lockfile.New(c.PidPath.ToString())
		if err != nil {
			// Should only happen if we didn't pass an absolute path
			return nil, err
		}
		var serverPid int
		if daemonProcess, err := lockFile.GetOwner(); errors.Is(err, lockfile.ErrDeadOwner) {
			// Report an error? We could technically race with another client trying to
			// start a daemon here.
			return nil, errors.Wrap(err, "pid file appears stale. If no daemon is running, please remove it")
		} else if os.IsNotExist(err) {
			// The pid file doesn't exist. Start a daemon
			pid, err := c.startDaemon()
			if err != nil {
				return nil, err
			}
			serverPid = pid
		} else {
			serverPid = daemonProcess.Pid
		}
		if err := c.waitForSocket(); errors.Is(err, ErrFailedToStart) {
			continue
		} else if err != nil {
			return nil, err
		}
		client, err := c.getClientConn()
		if err != nil {
			return nil, err
		}
		if err := c.sendHello(client); err == nil {
			// We connected and negotiated a version, we're all set
			return client, nil
		} else if errors.Is(err, errVersionMismatch) {
			// killLiveServer is responsible for closing the client
			if err := c.killLiveServer(client, serverPid); err != nil {
				return nil, err
			}
		} else if errors.Is(err, errConnectionFailure) {
			// close the client, see if we can kill the stale daemon
			_ = client.Close()
			if err := c.killDeadServer(serverPid); err != nil {
				return nil, err
			}
		} else if err != nil {
			// Some other error occurred, close the client and
			// report the error to the user
			_ = client.Close()
			return nil, err
		}
	}
	return nil, ErrTooManyAttempts
}

func (c *Connector) getClientConn() (*clientAndConn, error) {
	creds := insecure.NewCredentials()
	conn, err := grpc.Dial(c.addr(), grpc.WithTransportCredentials(creds))
	if err != nil {
		return nil, err
	}
	tc := server.NewTurboClient(conn)
	return &clientAndConn{
		TurboClient: tc,
		ClientConn:  conn,
	}, nil
}

func (c *Connector) sendHello(client server.TurboClient) error {
	_, err := client.Hello(c.Ctx, &server.HelloRequest{
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

// waitForSocket waits for the unix domain socket to appear
func (c *Connector) waitForSocket() error {
	// Note that we don't care if this is our daemon
	// or not. We started a process, but someone else could beat
	// use to listening. That's fine, we'll check the version
	// later.
	deadline := time.After(_socketPollTimeout)
	for !c.SockPath.FileExists() {
		select {
		case <-time.After(_socketPollInterval):
		case <-deadline:
			return ErrFailedToStart
		}
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
	err := cmd.Start()
	if err != nil {
		return 0, err
	}
	return cmd.Process.Pid, nil
}
