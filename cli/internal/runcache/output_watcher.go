package runcache

import (
	"context"

	"github.com/vercel/turbo/cli/internal/fs"
)

// OutputWatcher instances are responsible for tracking changes to task outputs
type OutputWatcher interface {
	// GetChangedOutputs returns which of the given globs have changed since the specified hash was last run
	GetChangedOutputs(ctx context.Context, hash string, repoRelativeOutputGlobs []string) ([]string, error)
	// NotifyOutputsWritten tells the watcher that the given globs have been cached with the specified hash
	NotifyOutputsWritten(ctx context.Context, hash string, repoRelativeOutputGlobs fs.TaskOutputs) error
}

// NoOpOutputWatcher implements OutputWatcher, but always considers every glob to have changed
type NoOpOutputWatcher struct{}

var _ OutputWatcher = (*NoOpOutputWatcher)(nil)

// GetChangedOutputs implements OutputWatcher.GetChangedOutputs.
// Since this is a no-op watcher, no tracking is done.
func (NoOpOutputWatcher) GetChangedOutputs(ctx context.Context, hash string, repoRelativeOutputGlobs []string) ([]string, error) {
	return repoRelativeOutputGlobs, nil
}

// NotifyOutputsWritten implements OutputWatcher.NotifyOutputsWritten.
// Since this is a no-op watcher, consider all globs to have changed
func (NoOpOutputWatcher) NotifyOutputsWritten(ctx context.Context, hash string, repoRelativeOutputGlobs fs.TaskOutputs) error {
	return nil
}
