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
	"github.com/vercel/turborepo/cli/internal/server"
	"github.com/vercel/turborepo/cli/internal/signals"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/test/grpc_testing"
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

type testRPCServer struct {
	grpc_testing.UnimplementedTestServiceServer
	registered chan struct{}
}

func (ts *testRPCServer) EmptyCall(ctx context.Context, req *grpc_testing.Empty) (*grpc_testing.Empty, error) {
	panic("intended to panic")
}

func (ts *testRPCServer) Register(grpcServer server.GRPCServer) {
	grpc_testing.RegisterTestServiceServer(grpcServer, ts)
	ts.registered <- struct{}{}
}

func newTestRPCServer() *testRPCServer {
	return &testRPCServer{
		registered: make(chan struct{}, 1),
	}
}

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
	repoRootRaw := fs.NewDir(t, "daemon-test")
	repoRoot := turbofs.UnsafeToAbsolutePath(repoRootRaw.Path())

	ts := newTestRPCServer()
	watcher := signals.NewWatcher()
	ctx, cancel := context.WithCancel(context.Background())

	d := &daemon{
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
		serverErr = d.runTurboServer(ts, watcher)
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
	repoRootRaw := fs.NewDir(t, "daemon-test")
	repoRoot := turbofs.UnsafeToAbsolutePath(repoRootRaw.Path())

	ts := newTestRPCServer()
	watcher := signals.NewWatcher()
	ctx, cancel := context.WithCancel(context.Background())

	d := &daemon{
		logger:     logger,
		repoRoot:   repoRoot,
		timeout:    5 * time.Millisecond,
		reqCh:      make(chan struct{}),
		timedOutCh: make(chan struct{}),
		ctx:        ctx,
		cancel:     cancel,
	}
	err := d.runTurboServer(ts, watcher)
	if !errors.Is(err, errInactivityTimeout) {
		t.Errorf("server error got %v, want %v", err, errInactivityTimeout)
	}
	_, ok := <-ctx.Done()
	if ok {
		t.Error("expected context to be done")
	}
}

func TestCaughtSignal(t *testing.T) {
	logger := hclog.Default()
	logger.SetLevel(hclog.Debug)
	repoRootRaw := fs.NewDir(t, "daemon-test")
	repoRoot := turbofs.UnsafeToAbsolutePath(repoRootRaw.Path())

	ts := newTestRPCServer()
	watcher := signals.NewWatcher()
	ctx, cancel := context.WithCancel(context.Background())

	d := &daemon{
		logger:     logger,
		repoRoot:   repoRoot,
		timeout:    5 * time.Second,
		reqCh:      make(chan struct{}),
		timedOutCh: make(chan struct{}),
		ctx:        ctx,
		cancel:     cancel,
	}
	errCh := make(chan error)
	go func() {
		err := d.runTurboServer(ts, watcher)
		errCh <- err
	}()
	<-ts.registered
	// If we've registered with the turboserver, we've registered all of our
	// signal handlers as well
	watcher.Close()

	pidPath := getPidFile(repoRoot)
	if pidPath.FileExists() {
		t.Errorf("expected to clean up %v, but it still exists", pidPath)
	}
	err := <-errCh
	assert.NilError(t, err, "runTurboServer")
	_, ok := <-ctx.Done()
	if ok {
		t.Error("expected context to be done")
	}
}

func TestCleanupOnPanic(t *testing.T) {
	logger := hclog.Default()
	logger.SetLevel(hclog.Debug)
	repoRootRaw := fs.NewDir(t, "daemon-test")
	repoRoot := turbofs.UnsafeToAbsolutePath(repoRootRaw.Path())

	ts := newTestRPCServer()
	watcher := signals.NewWatcher()
	ctx, cancel := context.WithCancel(context.Background())

	d := &daemon{
		logger:     logger,
		repoRoot:   repoRoot,
		timeout:    5 * time.Second,
		reqCh:      make(chan struct{}),
		timedOutCh: make(chan struct{}),
		ctx:        ctx,
		cancel:     cancel,
	}
	errCh := make(chan error)
	go func() {
		err := d.runTurboServer(ts, watcher)
		errCh <- err
	}()
	<-ts.registered

	creds := insecure.NewCredentials()
	sockFile := getUnixSocket(repoRoot)
	conn, err := grpc.Dial("unix://"+sockFile.ToString(), grpc.WithTransportCredentials(creds))
	assert.NilError(t, err, "Dial")

	client := grpc_testing.NewTestServiceClient(conn)
	_, err = client.EmptyCall(ctx, &grpc_testing.Empty{})
	if err == nil {
		t.Error("nil error")
	}
	// wait for the server to finish
	<-errCh

	pidPath := getPidFile(repoRoot)
	if pidPath.FileExists() {
		t.Errorf("expected to clean up %v, but it still exists", pidPath)
	}
}
