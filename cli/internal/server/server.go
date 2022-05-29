package server

import (
	"context"
	sync "sync"

	"github.com/fsnotify/fsnotify"
	"github.com/hashicorp/go-hclog"
	"github.com/pkg/errors"
	"github.com/vercel/turborepo/cli/internal/filewatcher"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/globwatcher"
	"google.golang.org/grpc"
	codes "google.golang.org/grpc/codes"
	status "google.golang.org/grpc/status"
)

// Server implements the GRPC serverside of TurboServer
// Note for the future: we don't yet make use of turbo.json
// or the package graph in the server. Once we do, we may need a
// layer of indirection between "the thing that responds to grpc requests"
// and "the thing that holds our persistent data structures" to handle
// changes in the underlying configuration.
type Server struct {
	UnimplementedTurboServer
	watcher      *filewatcher.FileWatcher
	globWatcher  *globwatcher.GlobWatcher
	turboVersion string
	closer       *closer
}

type closer struct {
	grpcServer *grpc.Server
	once       sync.Once
}

func (c *closer) close() {
	c.once.Do(func() {
		go func() {
			c.grpcServer.GracefulStop()
		}()
	})
}

// New returns a new instance of Server
func New(logger hclog.Logger, repoRoot fs.AbsolutePath, turboVersion string) (*Server, error) {
	watcher, err := fsnotify.NewWatcher()
	if err != nil {
		return nil, err
	}
	fileWatcher := filewatcher.New(logger.Named("FileWatcher"), repoRoot, watcher)
	globWatcher := globwatcher.New(logger.Named("GlobWatcher"), repoRoot)
	server := &Server{
		watcher:      fileWatcher,
		globWatcher:  globWatcher,
		turboVersion: turboVersion,
	}
	server.watcher.AddClient(globWatcher)
	if err := server.watcher.Start(); err != nil {
		return nil, errors.Wrapf(err, "watching %v", repoRoot)
	}
	return server, nil
}

// Close is used for shutting down this copy of the server
func (s *Server) Close() error {
	return s.watcher.Close()
}

// Register registers this server to respond to GRPC requests
func (s *Server) Register(grpcServer *grpc.Server) {
	s.closer = &closer{
		grpcServer: grpcServer,
	}
	RegisterTurboServer(grpcServer, s)
}

// NotifyOutputsWritten implements the NotifyOutputsWritten rpc from turbo.proto
func (s *Server) NotifyOutputsWritten(ctx context.Context, req *NotifyOutputsWrittenRequest) (*NotifyOutputsWrittenResponse, error) {
	err := s.globWatcher.WatchGlobs(req.Hash, req.OutputGlobs)
	if err != nil {
		return nil, err
	}
	return &NotifyOutputsWrittenResponse{}, nil
}

// GetChangedOutputs implements the GetChangedOutputs rpc from turbo.proto
func (s *Server) GetChangedOutputs(ctx context.Context, req *GetChangedOutputsRequest) (*GetChangedOutputsResponse, error) {
	changedGlobs, err := s.globWatcher.GetChangedGlobs(req.Hash, req.OutputGlobs)
	if err != nil {
		return nil, err
	}
	return &GetChangedOutputsResponse{
		ChangedOutputGlobs: changedGlobs,
	}, nil
}

// Hello implements the Hello rpc from turbo.proto
func (s *Server) Hello(ctx context.Context, req *HelloRequest) (*HelloResponse, error) {
	clientVersion := req.Version
	if clientVersion != s.turboVersion {
		err := status.Errorf(codes.FailedPrecondition, "version mismatch. Client %v Server %v", clientVersion, s.turboVersion)
		return nil, err
	}
	return &HelloResponse{}, nil
}

// Shutdown implements the Shutdown rpc from turbo.proto
func (s *Server) Shutdown(ctx context.Context, req *ShutdownRequest) (*ShutdownResponse, error) {
	if s.closer != nil {
		s.closer.close()
		return &ShutdownResponse{}, nil
	}
	err := status.Error(codes.NotFound, "shutdown mechanism not found")
	return nil, err
}
