package connector

import (
	"context"
	"errors"
	"net"
	"os/exec"
	"runtime"
	"strconv"
	"testing"

	"github.com/hashicorp/go-hclog"
	turbofs "github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/server"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/status"
	"google.golang.org/grpc/test/bufconn"
	"gotest.tools/v3/assert"
	"gotest.tools/v3/fs"
)

func testBin() string {
	if runtime.GOOS == "windows" {
		return "node.exe"
	}
	return "node"
}

func getUnixSocket(dir turbofs.AbsolutePath) turbofs.AbsolutePath {
	return dir.Join("turbod-test.sock")
}

func getPidFile(dir turbofs.AbsolutePath) turbofs.AbsolutePath {
	return dir.Join("turbod-test.pid")
}

func TestConnectAndHello_ConnectFails(t *testing.T) {
	logger := hclog.Default()
	dir := fs.NewDir(t, "daemon-test")
	dirPath := turbofs.UnsafeToAbsolutePath(dir.Path())
	err := dirPath.MkdirAll()
	assert.NilError(t, err, "MkdirAll")

	sockPath := getUnixSocket(dirPath)
	err = sockPath.EnsureDir()
	assert.NilError(t, err, "EnsureDir")
	// Simulate the socket already existing, with no live daemon
	err = sockPath.WriteFile([]byte("junk"), 0644)
	assert.NilError(t, err, "WriteFile")
	pidPath := getPidFile(dirPath)
	err = sockPath.EnsureDir()
	assert.NilError(t, err, "EnsureDir")
	ctx := context.Background()
	c := &Connector{
		Logger:   logger,
		Bin:      "nonexistent",
		Opts:     Opts{},
		SockPath: sockPath,
		PidPath:  pidPath,
		Ctx:      ctx,
	}
	conn, err := c.Connect()
	assert.NilError(t, err, "connect")
	defer func() { _ = conn.Close() }()
	err = c.sendHello(conn)
	assert.ErrorIs(t, err, errConnectionFailure)
}

func TestKillDeadServerNoPid(t *testing.T) {
	logger := hclog.Default()
	dir := fs.NewDir(t, "daemon-test")
	dirPath := turbofs.UnsafeToAbsolutePath(dir.Path())
	err := dirPath.MkdirAll()
	assert.NilError(t, err, "MkdirAll")

	sockPath := getUnixSocket(dirPath)
	pidPath := getPidFile(dirPath)
	err = sockPath.EnsureDir()
	assert.NilError(t, err, "EnsureDir")
	// Simulate the socket already existing, with no live daemon
	err = sockPath.WriteFile([]byte("junk"), 0644)
	assert.NilError(t, err, "WriteFile")
	ctx := context.Background()
	c := &Connector{
		Logger:   logger,
		Bin:      "nonexistent",
		Opts:     Opts{},
		SockPath: sockPath,
		PidPath:  pidPath,
		Ctx:      ctx,
	}

	err = c.killDeadServer()
	assert.NilError(t, err, "killDeadServer")
	stillExists := sockPath.FileExists()
	if stillExists {
		t.Error("sockPath still exists, expected it to be cleaned up")
	}
}

func TestKillDeadServerNoProcess(t *testing.T) {
	logger := hclog.Default()
	dir := fs.NewDir(t, "daemon-test")
	dirPath := turbofs.UnsafeToAbsolutePath(dir.Path())
	err := dirPath.MkdirAll()
	assert.NilError(t, err, "MkdirAll")

	sockPath := getUnixSocket(dirPath)
	pidPath := getPidFile(dirPath)
	err = sockPath.EnsureDir()
	assert.NilError(t, err, "EnsureDir")
	// Simulate the socket already existing, with no live daemon
	err = sockPath.WriteFile([]byte("junk"), 0644)
	assert.NilError(t, err, "WriteFile")
	err = pidPath.WriteFile([]byte("99999"), 0644)
	assert.NilError(t, err, "WriteFile")
	ctx := context.Background()
	c := &Connector{
		Logger:   logger,
		Bin:      "nonexistent",
		Opts:     Opts{},
		SockPath: sockPath,
		PidPath:  pidPath,
		Ctx:      ctx,
	}

	err = c.killDeadServer()
	assert.NilError(t, err, "killDeadServer")
	stillExists := sockPath.FileExists()
	if stillExists {
		t.Error("sockPath still exists, expected it to be cleaned up")
	}
	stillExists = pidPath.FileExists()
	if stillExists {
		t.Error("pidPath still exists, expected it to be cleaned up")
	}
}

func TestKillDeadServerWithProcess(t *testing.T) {
	logger := hclog.Default()
	dir := fs.NewDir(t, "daemon-test")
	dirPath := turbofs.UnsafeToAbsolutePath(dir.Path())
	err := dirPath.MkdirAll()
	assert.NilError(t, err, "MkdirAll")

	sockPath := getUnixSocket(dirPath)
	pidPath := getPidFile(dirPath)
	err = sockPath.EnsureDir()
	assert.NilError(t, err, "EnsureDir")
	// Simulate the socket already existing, with no live daemon
	err = sockPath.WriteFile([]byte("junk"), 0644)
	assert.NilError(t, err, "WriteFile")
	bin := testBin()
	cmd := exec.Command(bin)
	err = cmd.Start()
	assert.NilError(t, err, "cmd.Start")
	pid := cmd.Process.Pid
	if pid == 0 {
		t.Fatalf("failed to start process %v", bin)
	}

	err = pidPath.WriteFile([]byte(strconv.Itoa(pid)), 0644)
	assert.NilError(t, err, "WriteFile")
	ctx := context.Background()
	c := &Connector{
		Logger:   logger,
		Bin:      "nonexistent",
		Opts:     Opts{},
		SockPath: sockPath,
		PidPath:  pidPath,
		Ctx:      ctx,
	}

	err = c.killDeadServer()
	assert.NilError(t, err, "killDeadServer")
	stillExists := sockPath.FileExists()
	if stillExists {
		t.Error("sockPath still exists, expected it to be cleaned up")
	}
	stillExists = pidPath.FileExists()
	if stillExists {
		t.Error("pidPath still exists, expected it to be cleaned up")
	}
	err = cmd.Wait()
	exitErr := &exec.ExitError{}
	if !errors.As(err, &exitErr) {
		t.Errorf("expected an exit error from %v, got %v", bin, err)
	}
}

type mockServer struct {
	server.UnimplementedTurboServer
	helloErr     error
	shutdownResp *server.ShutdownResponse
}

func (s *mockServer) Shutdown(ctx context.Context, req *server.ShutdownRequest) (*server.ShutdownResponse, error) {
	return s.shutdownResp, nil
}

func (s *mockServer) Hello(ctx context.Context, req *server.HelloRequest) (*server.HelloResponse, error) {
	if req.Version == "" {
		return nil, errors.New("missing version")
	}
	return nil, s.helloErr
}

func TestKillLiveServer(t *testing.T) {
	logger := hclog.Default()
	dir := fs.NewDir(t, "daemon-test")
	dirPath := turbofs.UnsafeToAbsolutePath(dir.Path())
	err := dirPath.MkdirAll()
	assert.NilError(t, err, "MkdirAll")

	sockPath := getUnixSocket(dirPath)
	pidPath := getPidFile(dirPath)
	err = sockPath.EnsureDir()
	assert.NilError(t, err, "EnsureDir")
	// Simulate socket and pid existing
	err = sockPath.WriteFile([]byte("junk"), 0644)
	assert.NilError(t, err, "WriteFile")
	err = pidPath.WriteFile([]byte("99999"), 0644)
	assert.NilError(t, err, "WriteFile")

	ctx := context.Background()
	c := &Connector{
		Logger:       logger,
		Bin:          "nonexistent",
		Opts:         Opts{},
		SockPath:     sockPath,
		PidPath:      pidPath,
		Ctx:          ctx,
		TurboVersion: "some-version",
	}

	st := status.New(codes.FailedPrecondition, "version mismatch")
	mock := &mockServer{
		shutdownResp: &server.ShutdownResponse{},
		helloErr:     st.Err(),
	}
	lis := bufconn.Listen(1024 * 1024)
	grpcServer := grpc.NewServer()
	server.RegisterTurboServer(grpcServer, mock)
	go func(t *testing.T) {
		if err := grpcServer.Serve(lis); err != nil {
			t.Logf("server closed: %v", err)
		}
	}(t)

	conn, err := grpc.DialContext(ctx, "bufnet", grpc.WithContextDialer(func(ctx context.Context, s string) (net.Conn, error) {
		return lis.Dial()
	}), grpc.WithTransportCredentials(insecure.NewCredentials()))
	assert.NilError(t, err, "DialContext")
	turboClient := server.NewTurboClient(conn)
	client := &clientAndConn{
		TurboClient: turboClient,
		ClientConn:  conn,
	}
	err = c.sendHello(client)
	if !errors.Is(err, errVersionMismatch) {
		t.Errorf("sendHello error got %v, want %v", err, errVersionMismatch)
	}
	err = c.killLiveServer(client)
	assert.NilError(t, err, "killLiveServer")
	// Expect the pid file and socket files to have been cleaned up
	if pidPath.FileExists() {
		t.Errorf("expected pid file to have been deleted: %v", pidPath)
	}
	if sockPath.FileExists() {
		t.Errorf("expected socket file to have been deleted: %v", sockPath)
	}
}
