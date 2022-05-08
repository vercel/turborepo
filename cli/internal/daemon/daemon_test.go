package daemon

import (
	"context"
	"errors"
	"os"
	"sync"
	"testing"
	"time"

	"github.com/hashicorp/go-hclog"
	turbofs "github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/ui"
	"google.golang.org/grpc"
	"gotest.tools/v3/assert"
	"gotest.tools/v3/fs"
)

func TestDaemonDebounce(t *testing.T) {
	repoRootRaw := fs.NewDir(t, "daemon-test")
	repoRoot := turbofs.UnsafeToAbsolutePath(repoRootRaw.Path())

	sockPath := getUnixSocket(repoRoot)
	err := sockPath.EnsureDir()
	pidPath := getPidFile(repoRoot)
	assert.NilError(t, err, "EnsureDir")
	err = sockPath.WriteFile([]byte("junk"), 0644)
	assert.NilError(t, err, "WriteFile")

	d := &daemon{}
	_, err = d.debounceServers(sockPath, pidPath)
	if !errors.Is(err, errAlreadyRunning) {
		t.Errorf("debounce err got %v, want %v", err, errAlreadyRunning)
	}

	err = sockPath.Remove()
	assert.NilError(t, err, "Remove")
	lockFile, err := d.debounceServers(sockPath, pidPath)
	assert.NilError(t, err, "debounceServers")

	if !pidPath.FileExists() {
		t.Errorf("expected to create and lock %v", pidPath)
	}

	owner, err := lockFile.GetOwner()
	assert.NilError(t, err, "GetOwner")
	ourPid := os.Getpid()
	if owner.Pid != ourPid {
		t.Errorf("lock pid got %v, want %v", owner.Pid, ourPid)
	}
}

type testRPCServer struct{}

func (ts *testRPCServer) Register(grpcServer *grpc.Server) {}

func waitForFile(t *testing.T, filename turbofs.AbsolutePath, timeout time.Duration) {
	deadline := time.After(timeout)
outer:
	for !filename.FileExists() {
		select {
		case <-deadline:
			break outer
		case <-time.After(10 * time.Millisecond):
		}
	}
	if !filename.FileExists() {
		t.Errorf("timed out waiting for %v to exist after %v", filename, timeout)
	}
}

func TestDaemonLifecycle(t *testing.T) {
	logger := hclog.Default()
	logger.SetLevel(hclog.Debug)
	tui := ui.Default()
	repoRootRaw := fs.NewDir(t, "daemon-test")
	repoRoot := turbofs.UnsafeToAbsolutePath(repoRootRaw.Path())

	ts := &testRPCServer{}
	ctx, cancel := context.WithCancel(context.Background())

	d := &daemon{
		ui:         tui,
		logger:     logger,
		repoRoot:   repoRoot,
		timeout:    10 * time.Second,
		reqCh:      make(chan struct{}),
		timedOutCh: make(chan struct{}),
		ctx:        ctx,
		cancel:     cancel,
	}

	var serverErr error
	wg := &sync.WaitGroup{}
	wg.Add(1)
	go func() {
		serverErr = d.runTurboServer(ts)
		wg.Done()
	}()

	sockPath := getUnixSocket(repoRoot)
	waitForFile(t, sockPath, 3*time.Second)
	pidPath := getPidFile(repoRoot)
	waitForFile(t, pidPath, 1*time.Second)
	cancel()
	wg.Wait()
	assert.NilError(t, serverErr, "runTurboServer")
	if sockPath.FileExists() {
		t.Errorf("%v still exists, should have been cleaned up", sockPath)
	}
	if pidPath.FileExists() {
		t.Errorf("%v still exists, should have been cleaned up", sockPath)
	}
}

func TestTimeout(t *testing.T) {
	logger := hclog.Default()
	logger.SetLevel(hclog.Debug)
	tui := ui.Default()
	repoRootRaw := fs.NewDir(t, "daemon-test")
	repoRoot := turbofs.UnsafeToAbsolutePath(repoRootRaw.Path())

	ts := &testRPCServer{}
	ctx, cancel := context.WithCancel(context.Background())

	d := &daemon{
		ui:         tui,
		logger:     logger,
		repoRoot:   repoRoot,
		timeout:    5 * time.Millisecond,
		reqCh:      make(chan struct{}),
		timedOutCh: make(chan struct{}),
		ctx:        ctx,
		cancel:     cancel,
	}
	err := d.runTurboServer(ts)
	if !errors.Is(err, errInactivityTimeout) {
		t.Errorf("server error got %v, want %v", err, errInactivityTimeout)
	}
	_, ok := <-ctx.Done()
	if ok {
		t.Error("expected context to be done")
	}
}
