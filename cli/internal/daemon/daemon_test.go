package daemon

import (
	"context"
	"errors"
	"os/exec"
	"runtime"
	"strconv"
	"sync"
	"testing"
	"time"

	"github.com/hashicorp/go-hclog"
	"github.com/nightlyone/lockfile"
	turbofs "github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/ui"
	"google.golang.org/grpc"
	"gotest.tools/v3/assert"
	"gotest.tools/v3/fs"
)

func testBin() string {
	if runtime.GOOS == "windows" {
		return "node.exe"
	}
	return "node"
}

func TestDaemonDebounce(t *testing.T) {
	repoRootRaw := fs.NewDir(t, "daemon-test")
	repoRoot := turbofs.UnsafeToAbsolutePath(repoRootRaw.Path())

	pidPath := getPidFile(repoRoot)
	err := pidPath.EnsureDir()
	assert.NilError(t, err, "EnsureDir")
	// Write a garbage pid
	// err = pidPath.WriteFile([]byte("99999999"), 0644)
	// assert.NilError(t, err, "WriteFile")

	d := &daemon{}
	// the lockfile library handles removing pids from dead owners
	_, err = d.debounceServers(pidPath)
	assert.NilError(t, err, "debounceServers")

	// Start up a node process and fake a pid file for it.
	// Ensure that we can't start the daemon while the node process is live
	bin := testBin()
	node := exec.Command(bin)
	err = node.Start()
	assert.NilError(t, err, "Start")
	stopNode := func() error {
		if err := node.Process.Kill(); err != nil {
			return err
		}
		// We expect an error from node, we just sent a kill signal
		_ = node.Wait()
		return nil
	}
	// In case we fail the test, still try to kill the node process
	t.Cleanup(func() { _ = stopNode() })
	nodePid := node.Process.Pid
	err = pidPath.WriteFile([]byte(strconv.Itoa(nodePid)), 0644)
	assert.NilError(t, err, "WriteFile")

	_, err = d.debounceServers(pidPath)
	assert.ErrorIs(t, err, lockfile.ErrBusy)

	// Stop the node process, but leave the pid file there
	// This simulates a crash
	err = stopNode()
	assert.NilError(t, err, "stopNode")
	// the lockfile library handles removing pids from dead owners
	_, err = d.debounceServers(pidPath)
	assert.NilError(t, err, "debounceServers")
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
