package runcache

// OutputWatcher instances are responsible for tracking changes to task outputs
type OutputWatcher interface {
	// GetChangedOutputs returns which of the given globs have changed since the specified hash was last run
	GetChangedOutputs(hash string, repoRelativeOutputGlobs []string) ([]string, error)
	// NotifyOutputsWritten tells the watcher that the given globs have been cached with the specified hash
	NotifyOutputsWritten(hash string, repoRelativeOutputGlobs []string) error
}

// NoOpOutputWatcher implements OutputWatcher, but always considers every glob to have changed
type NoOpOutputWatcher struct{}

var _ OutputWatcher = &NoOpOutputWatcher{}

// GetChangedOutputs implements OutputWatcher.GetChangedOutputs.
// Since this is a no-op watcher, no tracking is done.
func (NoOpOutputWatcher) GetChangedOutputs(hash string, repoRelativeOutputGlobs []string) ([]string, error) {
	return repoRelativeOutputGlobs, nil
}

// NotifyOutputsWritten implements OutputWatcher.NotifyOutputsWritten.
// Since this is a no-op watcher, consider all globs to have changed
func (NoOpOutputWatcher) NotifyOutputsWritten(hash string, repoRelativeOutputGlobs []string) error {
	return nil
}
