// Package daemonclient is a wrapper around a grpc client
// to talk to turbod
package daemonclient

import (
	"context"

	"github.com/vercel/turborepo/cli/internal/daemon/connector"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/server"
)

// DaemonClient provides access to higher-level functionality from the daemon to a turbo run.
type DaemonClient struct {
	client *connector.Client
	ctx    context.Context
}

// Status provides details about the daemon's status
type Status struct {
	UptimeMs uint64          `json:"uptimeMs"`
	LogFile  fs.AbsolutePath `json:"logFile"`
	PidFile  fs.AbsolutePath `json:"pidFile"`
	SockFile fs.AbsolutePath `json:"sockFile"`
}

// New creates a new instance of a DaemonClient.
func New(ctx context.Context, client *connector.Client) *DaemonClient {
	return &DaemonClient{
		client: client,
		ctx:    ctx,
	}
}

// GetChangedOutputs implements runcache.OutputWatcher.GetChangedOutputs
func (d *DaemonClient) GetChangedOutputs(hash string, repoRelativeOutputGlobs []string) ([]string, error) {
	resp, err := d.client.GetChangedOutputs(d.ctx, &server.GetChangedOutputsRequest{
		Hash:        hash,
		OutputGlobs: repoRelativeOutputGlobs,
	})
	if err != nil {
		return nil, err
	}
	return resp.ChangedOutputGlobs, nil
}

// NotifyOutputsWritten implements runcache.OutputWatcher.NotifyOutputsWritten
func (d *DaemonClient) NotifyOutputsWritten(hash string, repoRelativeOutputGlobs []string) error {
	_, err := d.client.NotifyOutputsWritten(d.ctx, &server.NotifyOutputsWrittenRequest{
		Hash:        hash,
		OutputGlobs: repoRelativeOutputGlobs,
	})
	return err
}

// Status returns the DaemonStatus from the daemon
func (d *DaemonClient) Status() (*Status, error) {
	resp, err := d.client.Status(d.ctx, &server.StatusRequest{})
	if err != nil {
		return nil, err
	}
	daemonStatus := resp.DaemonStatus
	return &Status{
		UptimeMs: daemonStatus.UptimeMsec,
		LogFile:  d.client.LogPath,
		PidFile:  d.client.PidPath,
		SockFile: d.client.SockPath,
	}, nil
}
