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

// If the socket and pid file exist, dial and call Hello
//   If can't connect:
//     try to kill the process and restart it
//   If there's a version mismatch, attempt to kill and restart
//   If that times out, report an error
// If the socket and pid don't exist, try starting the deamon
// After _maxAttempts, give up and return an err.
// The error should contain enough diagnostic information to
// investigate (pid file path, socket file path)
const _maxAttempts = 3

var (
	_shutdownDeadline     = 1 * time.Second
	_shutdownPollInterval = 50 * time.Millisecond
)

// killLiveServer tells a running server to shut down. This method is also responsible
// for closing this connection
func (c *Connector) killLiveServer(client *clientAndConn) error {
	defer func() { _ = client.Close() }()

	_, err := client.Shutdown(c.Ctx, &server.ShutdownRequest{})
	if err != nil {
		c.Logger.Error(fmt.Sprintf("failed to shutdown running daemon. attempting to force it closed: %v", err))
		return c.killDeadServer()
	}
	// Wait for the server to gracefully exit
	deadline := time.After(_shutdownDeadline)
outer:
	for c.PidPath.FileExists() {
		select {
		case <-deadline:
			break outer
		case <-time.After(_shutdownPollInterval):
		}
	}
	if c.PidPath.FileExists() {
		c.Logger.Error(fmt.Sprintf("daemon did not exit after %v, attempting to force it closed", _shutdownDeadline.String()))
		return c.killDeadServer()
	}
	return nil
}

func (c *Connector) killDeadServer() error {
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
		// we have a process that we need to kill
		// TODO(gsoltis): graceful kill? the process is already not responding to requests
		if err := process.Kill(); err != nil {
			return err
		}
	} else if errors.Is(err, os.ErrNotExist) {
		// There's no pid file. Just remove the socket file and move on
		return c.SockPath.Remove()
	}
	// If we've either killed the server or it wasn't running to begin with
	if err == nil || errors.Is(err, lockfile.ErrDeadOwner) {
		if err := c.SockPath.Remove(); err != nil && !errors.Is(err, os.ErrNotExist) {
			return err
		}
		if err := c.PidPath.Remove(); err != nil && !errors.Is(err, os.ErrNotExist) {
			return err
		}
		return nil
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

func (c *Connector) connectInternal() (Client, error) {
	if !c.SockPath.FileExists() {
		if err := c.startDaemon(); err != nil {
			return nil, err
		}
	}
	attempts := 0
	var client *clientAndConn
	var err error
	for client == nil && attempts < _maxAttempts {
		client, err = c.getClientConn()
		if err != nil {
			return nil, err
		}
		err = c.sendHello(client)
		if errors.Is(err, errVersionMismatch) {
			if err := c.killLiveServer(client); err != nil {
				return nil, err
			}
			attempts++
		} else if errors.Is(err, errConnectionFailure) {
			if err := c.killDeadServer(); err != nil {
				return nil, err
			}
			attempts++
		}
		// TODO: ensure that we try starting the server again after trying to kill it
	}
	if attempts == _maxAttempts {
		return nil, ErrTooManyAttempts
	}
	return client, nil
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

func (c *Connector) startDaemon() error {
	args := []string{"daemon"}
	if c.Opts.ServerTimeout != 0 {
		args = append(args, fmt.Sprintf("--idle-time=%v", c.Opts.ServerTimeout.String()))
	}
	c.Logger.Debug(fmt.Sprintf("starting turbod binary %v", c.Bin))
	cmd := exec.Command(c.Bin, args...)
	err := cmd.Start()
	if err != nil {
		return err
	}
	for i := 0; i < 150; i++ {
		<-time.After(20 * time.Millisecond)
		if c.SockPath.FileExists() {
			return nil
		}
	}
	return ErrFailedToStart
}
