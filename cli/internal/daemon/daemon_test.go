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
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/server"
	"github.com/vercel/turbo/cli/internal/signals"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/test/grpc_testing"
	"gotest.tools/v3/assert"
)

// testBin returns a platform-appropriate node binary.
// We need some process to be running and findable by the
// lockfile library, and we don't particularly care what it is.
// Since node is required for turbo development, it makes a decent
// candidate.
func testBin() string {
	if runtime.GOOS == "windows" {
		return "node.exe"
	}
	return "node"
}

func TestPidFileLock(t *testing.T) {
	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())

	pidPath := getPidFile(repoRoot)
	// the lockfile library handles removing pids from dead owners
	_, err := tryAcquirePidfileLock(pidPath)
	assert.NilError(t, err, "acquirePidLock")

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

	_, err = tryAcquirePidfileLock(pidPath)
	assert.ErrorIs(t, err, lockfile.ErrBusy)

	// Stop the node process, but leave the pid file there
	// This simulates a crash
	err = stopNode()
	assert.NilError(t, err, "stopNode")
	// the lockfile library handles removing pids from dead owners
	_, err = tryAcquirePidfileLock(pidPath)
	assert.NilError(t, err, "acquirePidLock")
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

func waitForFile(t *testing.T, filename turbopath.AbsoluteSystemPath, timeout time.Duration) {
	t.Helper()
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
	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())

	ts := newTestRPCServer()
	watcher := signals.NewWatcher()
	ctx, cancel := context.WithCancel(context.Background())

	d := &daemon{
		logger:     logger,
		repoRoot:   repoRoot,
		timeout:    10 * time.Second,
		reqCh:      make(chan struct{}),
		timedOutCh: make(chan struct{}),
	}

	var serverErr error
	wg := &sync.WaitGroup{}
	wg.Add(1)
	go func() {
		serverErr = d.runTurboServer(ctx, ts, watcher)
		wg.Done()
	}()

	sockPath := getUnixSocket(repoRoot)
	waitForFile(t, sockPath, 30*time.Second)
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
	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())

	ts := newTestRPCServer()
	watcher := signals.NewWatcher()
	ctx := context.Background()

	d := &daemon{
		logger:     logger,
		repoRoot:   repoRoot,
		timeout:    5 * time.Millisecond,
		reqCh:      make(chan struct{}),
		timedOutCh: make(chan struct{}),
	}
	err := d.runTurboServer(ctx, ts, watcher)
	if !errors.Is(err, errInactivityTimeout) {
		t.Errorf("server error got %v, want %v", err, errInactivityTimeout)
	}
}

func TestCaughtSignal(t *testing.T) {
	logger := hclog.Default()
	logger.SetLevel(hclog.Debug)
	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())

	ts := newTestRPCServer()
	watcher := signals.NewWatcher()
	ctx := context.Background()

	d := &daemon{
		logger:     logger,
		repoRoot:   repoRoot,
		timeout:    5 * time.Second,
		reqCh:      make(chan struct{}),
		timedOutCh: make(chan struct{}),
	}
	errCh := make(chan error)
	go func() {
		err := d.runTurboServer(ctx, ts, watcher)
		errCh <- err
	}()
	<-ts.registered
	// grpc doesn't provide a signal to know when the server is serving.
	// So while this call to Close can race with the call to grpc.Server.Serve, if we've
	// registered with the turboserver, we've registered all of our
	// signal handlers as well. We just may or may not be serving when Close()
	// is called. It shouldn't matter for the purposes of this test:
	// Either we are serving, and Serve will return with nil when GracefulStop is
	// called, or we aren't serving yet, and the subsequent call to Serve will
	// immediately return with grpc.ErrServerStopped. So, both nil and grpc.ErrServerStopped
	// are acceptable outcomes for runTurboServer. Any other error, or a timeout, is a
	// failure.
	watcher.Close()

	err := <-errCh
	pidPath := getPidFile(repoRoot)
	if pidPath.FileExists() {
		t.Errorf("expected to clean up %v, but it still exists", pidPath)
	}
	// We'll either get nil or ErrServerStopped, depending on whether
	// or not we close the signal watcher before grpc.Server.Serve was
	// called.
	if err != nil && !errors.Is(err, grpc.ErrServerStopped) {
		t.Errorf("runTurboServer got err %v, want nil or ErrServerStopped", err)
	}
}

func TestCleanupOnPanic(t *testing.T) {
	logger := hclog.Default()
	logger.SetLevel(hclog.Debug)
	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())

	ts := newTestRPCServer()
	watcher := signals.NewWatcher()
	ctx := context.Background()

	d := &daemon{
		logger:     logger,
		repoRoot:   repoRoot,
		timeout:    5 * time.Second,
		reqCh:      make(chan struct{}),
		timedOutCh: make(chan struct{}),
	}
	errCh := make(chan error)
	go func() {
		err := d.runTurboServer(ctx, ts, watcher)
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
