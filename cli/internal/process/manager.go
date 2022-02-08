package process

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"sync"
	"time"

	"github.com/hashicorp/go-hclog"
)

// ErrClosing is returned when the process manager is in the process of closing,
// meaning that no more child processes can be Exec'd, and existing, non-failed
// child processes will be stopped with this error.
var ErrClosing = errors.New("process manager is already closing")

// ChildExit is returned when a child process exits with a non-zero exit code
type ChildExit struct {
	ExitCode int
	Command  string
}

func (ce *ChildExit) Error() string {
	return fmt.Sprintf("command %s exited (%d)", ce.Command, ce.ExitCode)
}

// Manager tracks all of the child processes that have been spawned
type Manager struct {
	done     bool
	children map[*Child]struct{}
	mu       sync.Mutex
	doneCh   chan struct{}
	logger   hclog.Logger
}

// NewManager creates a new properly-initialized Manager instance
func NewManager(logger hclog.Logger) *Manager {
	return &Manager{
		children: make(map[*Child]struct{}),
		doneCh:   make(chan struct{}),
		logger:   logger,
	}
}

// Exec spawns a child process to run the given command, then blocks
// until it completes. Returns a nil error if the child process finished
// successfully, ErrClosing if the manager closed during execution, and
// a ChildExit error if the child process exited with a non-zero exit code.
func (m *Manager) Exec(cmd *exec.Cmd) error {
	m.mu.Lock()
	if m.done {
		m.mu.Unlock()
		return ErrClosing
	}

	child, err := newChild(NewInput{
		Cmd: cmd,
		// Run forever by default
		Timeout: 0,
		// When it's time to exit, give a 10 second timeout
		KillTimeout: 10 * time.Second,
		// Send SIGINT to stop children
		KillSignal: os.Interrupt,
		Logger:     m.logger,
	})
	if err != nil {
		return err
	}

	m.children[child] = struct{}{}
	m.mu.Unlock()
	err = child.Start()
	if err != nil {
		m.mu.Lock()
		delete(m.children, child)
		m.mu.Unlock()
		return err
	}
	err = nil
	exitCode, ok := <-child.ExitCh()
	if !ok {
		err = ErrClosing
	} else if exitCode != ExitCodeOK {
		err = &ChildExit{
			ExitCode: exitCode,
			Command:  child.Command(),
		}
	}

	m.mu.Lock()
	delete(m.children, child)
	m.mu.Unlock()
	return err
}

// Close sends SIGINT to all child processes if it hasn't been done yet,
// and in either case blocks until they all exit or timeout
func (m *Manager) Close() {
	m.mu.Lock()
	if m.done {
		m.mu.Unlock()
		<-m.doneCh
		return
	}
	wg := sync.WaitGroup{}
	m.done = true
	for child := range m.children {
		child := child
		wg.Add(1)
		go func() {
			child.Stop()
			wg.Done()
		}()
	}
	m.mu.Unlock()
	wg.Wait()
	close(m.doneCh)
}
