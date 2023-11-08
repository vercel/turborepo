package runcache

import (
	"context"

	"github.com/vercel/turbo/cli/internal/fs/hash"
)

// OutputWatcher instances are responsible for tracking changes to task outputs
type OutputWatcher interface {
	// GetChangedOutputs returns which of the given globs have changed since the specified hash was last run
	GetChangedOutputs(ctx context.Context, hash string, repoRelativeOutputGlobs []string) ([]string, int, error)
	// NotifyOutputsWritten tells the watcher that the given globs have been cached with the specified hash
	NotifyOutputsWritten(ctx context.Context, hash string, repoRelativeOutputGlobs hash.TaskOutputs, timeSaved int) error
}

// NoOpOutputWatcher implements OutputWatcher, but always considers every glob to have changed
type NoOpOutputWatcher struct{}

var _ OutputWatcher = (*NoOpOutputWatcher)(nil)

// GetChangedOutputs implements OutputWatcher.GetChangedOutputs.
// Since this is a no-op watcher, no tracking is done.
func (NoOpOutputWatcher) GetChangedOutputs(_ context.Context, _ string, repoRelativeOutputGlobs []string) ([]string, int, error) {
	return repoRelativeOutputGlobs, 0, nil
}

// NotifyOutputsWritten implements OutputWatcher.NotifyOutputsWritten.
// Since this is a no-op watcher, consider all globs to have changed
func (NoOpOutputWatcher) NotifyOutputsWritten(_ context.Context, _ string, _ hash.TaskOutputs, _ int) error {
	return nil
}
