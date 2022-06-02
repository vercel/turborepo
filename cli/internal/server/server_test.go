package server

import (
	"context"
	"testing"
	"time"

	"github.com/hashicorp/go-hclog"
	"google.golang.org/grpc"
	"gotest.tools/v3/assert"
	"gotest.tools/v3/fs"

	turbofs "github.com/vercel/turborepo/cli/internal/fs"
)

type mockGrpc struct {
	stopped chan struct{}
}

func (m *mockGrpc) GracefulStop() {
	close(m.stopped)
}

func (m *mockGrpc) RegisterService(desc *grpc.ServiceDesc, impl interface{}) {}

func TestDeleteRepoRoot(t *testing.T) {
	logger := hclog.Default()
	repoRootRaw := fs.NewDir(t, "server-test")
	repoRoot := turbofs.UnsafeToAbsolutePath(repoRootRaw.Path())

	grpcServer := &mockGrpc{
		stopped: make(chan struct{}),
	}

	s, err := New(logger, repoRoot, "some-version", "/log/file/path")
	assert.NilError(t, err, "New")
	s.Register(grpcServer)

	// Delete the repo root, ensure that GracefulStop got called
	err = repoRoot.Remove()
	assert.NilError(t, err, "Remove")

	select {
	case <-grpcServer.stopped:
	case <-time.After(2 * time.Second):
		t.Error("timed out waiting for graceful stop to be called")
	}
}

func TestShutdown(t *testing.T) {
	logger := hclog.Default()
	repoRootRaw := fs.NewDir(t, "server-test")
	repoRoot := turbofs.UnsafeToAbsolutePath(repoRootRaw.Path())

	grpcServer := &mockGrpc{
		stopped: make(chan struct{}),
	}

	s, err := New(logger, repoRoot, "some-version", "/log/file/path")
	assert.NilError(t, err, "New")
	s.Register(grpcServer)

	ctx := context.Background()
	_, err = s.Shutdown(ctx, &ShutdownRequest{})
	assert.NilError(t, err, "Shutdown")
	// Ensure that graceful stop gets called
	select {
	case <-grpcServer.stopped:
	case <-time.After(2 * time.Second):
		t.Error("timed out waiting for graceful stop to be called")
	}
}
