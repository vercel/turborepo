package process

/**
 * Code in this file is based on the source code at
 * https://github.com/hashicorp/consul-template/tree/3ea7d99ad8eff17897e0d63dac86d74770170bb8/child/child.go
 *
 * Major changes include removing the ability to restart a child process,
 * requiring a fully-formed exec.Cmd to be passed in, and including cmd.Dir
 * in the description of a child process.
 */

import (
	"errors"
	"fmt"
	"math/rand"
	"os"
	"os/exec"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/hashicorp/go-hclog"
)

func init() {
	// Seed the default rand Source with current time to produce better random
	// numbers used with splay
	rand.Seed(time.Now().UnixNano())
}

var (
	// ErrMissingCommand is the error returned when no command is specified
	// to run.
	ErrMissingCommand = errors.New("missing command")

	// ExitCodeOK is the default OK exit code.
	ExitCodeOK = 0

	// ExitCodeError is the default error code returned when the child exits with
	// an error without a more specific code.
	ExitCodeError = 127
)

// Child is a wrapper around a child process which can be used to send signals
// and manage the processes' lifecycle.
type Child struct {
	sync.RWMutex

	timeout time.Duration

	killSignal  os.Signal
	killTimeout time.Duration

	splay time.Duration

	// cmd is the actual child process under management.
	cmd *exec.Cmd

	// exitCh is the channel where the processes exit will be returned.
	exitCh chan int

	// stopLock is the mutex to lock when stopping. stopCh is the circuit breaker
	// to force-terminate any waiting splays to kill the process now. stopped is
	// a boolean that tells us if we have previously been stopped.
	stopLock sync.RWMutex
	stopCh   chan struct{}
	stopped  bool

	// whether to set process group id or not (default on)
	setpgid bool

	Label string

	logger hclog.Logger
}

// NewInput is input to the NewChild function.
type NewInput struct {
	// Cmd is the unstarted, preconfigured command to run
	Cmd *exec.Cmd

	// Timeout is the maximum amount of time to allow the command to execute. If
	// set to 0, the command is permitted to run infinitely.
	Timeout time.Duration

	// KillSignal is the signal to send to gracefully kill this process. This
	// value may be nil.
	KillSignal os.Signal

	// KillTimeout is the amount of time to wait for the process to gracefully
	// terminate before force-killing.
	KillTimeout time.Duration

	// Splay is the maximum random amount of time to wait before sending signals.
	// This option helps reduce the thundering herd problem by effectively
	// sleeping for a random amount of time before sending the signal. This
	// prevents multiple processes from all signaling at the same time. This value
	// may be zero (which disables the splay entirely).
	Splay time.Duration

	// Logger receives debug log lines about the process state and transitions
	Logger hclog.Logger
}

// New creates a new child process for management with high-level APIs for
// sending signals to the child process, restarting the child process, and
// gracefully terminating the child process.
func newChild(i NewInput) (*Child, error) {
	// exec.Command prepends the command to be run to the arguments list, so
	// we only need the arguments here, it will include the command itself.
	label := fmt.Sprintf("(%v) %v", i.Cmd.Dir, strings.Join(i.Cmd.Args, " "))
	child := &Child{
		cmd:         i.Cmd,
		timeout:     i.Timeout,
		killSignal:  i.KillSignal,
		killTimeout: i.KillTimeout,
		splay:       i.Splay,
		stopCh:      make(chan struct{}, 1),
		setpgid:     true,
		Label:       label,
		logger:      i.Logger.Named(label),
	}

	return child, nil
}

// ExitCh returns the current exit channel for this child process. This channel
// may change if the process is restarted, so implementers must not cache this
// value.
func (c *Child) ExitCh() <-chan int {
	c.RLock()
	defer c.RUnlock()
	return c.exitCh
}

// Pid returns the pid of the child process. If no child process exists, 0 is
// returned.
func (c *Child) Pid() int {
	c.RLock()
	defer c.RUnlock()
	return c.pid()
}

// Command returns the human-formatted command with arguments.
func (c *Child) Command() string {
	return c.Label
}

// Start starts and begins execution of the child process. A buffered channel
// is returned which is where the command's exit code will be returned upon
// exit. Any errors that occur prior to starting the command will be returned
// as the second error argument, but any errors returned by the command after
// execution will be returned as a non-zero value over the exit code channel.
func (c *Child) Start() error {
	// log.Printf("[INFO] (child) spawning: %s", c.Command())
	c.Lock()
	defer c.Unlock()
	return c.start()
}

// Signal sends the signal to the child process, returning any errors that
// occur.
func (c *Child) Signal(s os.Signal) error {
	c.logger.Debug("receiving signal %q", s.String())
	c.RLock()
	defer c.RUnlock()
	return c.signal(s)
}

// Kill sends the kill signal to the child process and waits for successful
// termination. If no kill signal is defined, the process is killed with the
// most aggressive kill signal. If the process does not gracefully stop within
// the provided KillTimeout, the process is force-killed. If a splay was
// provided, this function will sleep for a random period of time between 0 and
// the provided splay value to reduce the thundering herd problem. This function
// does not return any errors because it guarantees the process will be dead by
// the return of the function call.
func (c *Child) Kill() {
	c.logger.Debug("killing process")
	c.Lock()
	defer c.Unlock()
	c.kill(false)
}

// Stop behaves almost identical to Kill except it suppresses future processes
// from being started by this child and it prevents the killing of the child
// process from sending its value back up the exit channel. This is useful
// when doing a graceful shutdown of an application.
func (c *Child) Stop() {
	c.internalStop(false)
}

// StopImmediately behaves almost identical to Stop except it does not wait
// for any random splay if configured. This is used for performing a fast
// shutdown of consul-template and its children when a kill signal is received.
func (c *Child) StopImmediately() {
	c.internalStop(true)
}

func (c *Child) internalStop(immediately bool) {
	c.Lock()
	defer c.Unlock()

	c.stopLock.Lock()
	defer c.stopLock.Unlock()
	if c.stopped {
		return
	}
	c.kill(immediately)
	close(c.stopCh)
	c.stopped = true
}

func (c *Child) start() error {
	setSetpgid(c.cmd, c.setpgid)
	if err := c.cmd.Start(); err != nil {
		return err
	}

	// Create a new exitCh so that previously invoked commands (if any) don't
	// cause us to exit, and start a goroutine to wait for that process to end.
	exitCh := make(chan int, 1)
	go func() {
		var code int
		// It's possible that kill is called before we even
		// manage to get here. Make sure we still have a valid
		// cmd before waiting on it.
		c.RLock()
		var cmd = c.cmd
		c.RUnlock()
		var err error
		if cmd != nil {
			err = cmd.Wait()
		}
		if err == nil {
			code = ExitCodeOK
		} else {
			code = ExitCodeError
			if exiterr, ok := err.(*exec.ExitError); ok {
				if status, ok := exiterr.Sys().(syscall.WaitStatus); ok {
					code = status.ExitStatus()
				}
			}
		}

		// If the child is in the process of killing, do not send a response back
		// down the exit channel.
		c.stopLock.RLock()
		defer c.stopLock.RUnlock()
		if !c.stopped {
			select {
			case <-c.stopCh:
			case exitCh <- code:
			}
		}

		close(exitCh)
	}()

	c.exitCh = exitCh

	// If a timeout was given, start the timer to wait for the child to exit
	if c.timeout != 0 {
		select {
		case code := <-exitCh:
			if code != 0 {
				return fmt.Errorf(
					"command exited with a non-zero exit status:\n"+
						"\n"+
						"    %s\n"+
						"\n"+
						"This is assumed to be a failure. Please ensure the command\n"+
						"exits with a zero exit status.",
					c.Command(),
				)
			}
		case <-time.After(c.timeout):
			// Force-kill the process
			c.stopLock.Lock()
			defer c.stopLock.Unlock()
			if c.cmd != nil && c.cmd.Process != nil {
				c.cmd.Process.Kill()
			}

			return fmt.Errorf(
				"command did not exit within %q:\n"+
					"\n"+
					"    %s\n"+
					"\n"+
					"Commands must exit in a timely manner in order for processing to\n"+
					"continue. Consider using a process supervisor or utilizing the\n"+
					"built-in exec mode instead.",
				c.timeout,
				c.Command(),
			)
		}
	}

	return nil
}

func (c *Child) pid() int {
	if !c.running() {
		return 0
	}
	return c.cmd.Process.Pid
}

func (c *Child) signal(s os.Signal) error {
	if !c.running() {
		return nil
	}

	sig, ok := s.(syscall.Signal)
	if !ok {
		return fmt.Errorf("bad signal: %s", s)
	}
	pid := c.cmd.Process.Pid
	if c.setpgid {
		// kill takes negative pid to indicate that you want to use gpid
		pid = -(pid)
	}
	// cross platform way to signal process/process group
	p, err := os.FindProcess(pid)
	if err != nil {
		return err
	}
	return p.Signal(sig)
}

// kill sends the signal to kill the process using the configured signal
// if set, else the default system signal
func (c *Child) kill(immediately bool) {

	if !c.running() {
		c.logger.Debug("Kill() called but process dead; not waiting for splay.")
		return
	} else if immediately {
		c.logger.Debug("Kill() called but performing immediate shutdown; not waiting for splay.")
	} else {
		c.logger.Debug("Kill(%v) called", immediately)
		select {
		case <-c.stopCh:
		case <-c.randomSplay():
		}
	}

	var exited bool
	defer func() {
		if !exited {
			c.logger.Debug("PKill")
			c.cmd.Process.Kill()
		}
		c.cmd = nil
	}()

	if c.killSignal == nil {
		return
	}

	if err := c.signal(c.killSignal); err != nil {
		c.logger.Debug("Kill failed: %s", err)
		if processNotFoundErr(err) {
			exited = true // checked in defer
		}
		return
	}

	killCh := make(chan struct{}, 1)
	go func() {
		defer close(killCh)
		c.cmd.Process.Wait()
	}()

	select {
	case <-c.stopCh:
	case <-killCh:
		exited = true
	case <-time.After(c.killTimeout):
		c.logger.Debug("timeout")
	}
}

func (c *Child) running() bool {
	select {
	case <-c.exitCh:
		return false
	default:
	}
	return c.cmd != nil && c.cmd.Process != nil
}

func (c *Child) randomSplay() <-chan time.Time {
	if c.splay == 0 {
		return time.After(0)
	}

	ns := c.splay.Nanoseconds()
	offset := rand.Int63n(ns)
	t := time.Duration(offset)

	c.logger.Debug("waiting %.2fs for random splay", t.Seconds())

	return time.After(t)
}
