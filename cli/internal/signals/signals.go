package signals

import (
	"os"
	"os/signal"
	"sync"
	"syscall"
)

// Watcher watches for signals delivered to this process and provides
// the opportunity for turbo to run cleanup
type Watcher struct {
	doneCh  chan struct{}
	closed  bool
	mu      sync.Mutex
	closers []func()
}

// AddOnClose registers a cleanup handler to run when a signal is received
func (w *Watcher) AddOnClose(closer func()) {
	w.mu.Lock()
	defer w.mu.Unlock()
	w.closers = append(w.closers, closer)
}

// Close runs the cleanup handlers registered with this watcher
func (w *Watcher) Close() {
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.closed {
		return
	}
	w.closed = true
	for _, closer := range w.closers {
		closer()
	}
	w.closers = nil
	close(w.doneCh)
}

// Done returns a channel that will be closed after all of the cleanup
// handlers have been run.
func (w *Watcher) Done() <-chan struct{} {
	return w.doneCh
}

// NewWatcher returns a new Watcher instance for watching signals.
func NewWatcher() *Watcher {
	// TODO: platform specific signals to watch for?
	signalCh := make(chan os.Signal, 1)
	signal.Notify(signalCh, os.Interrupt, syscall.SIGTERM, syscall.SIGQUIT)
	w := &Watcher{
		doneCh: make(chan struct{}),
	}
	go func() {
		<-signalCh
		w.Close()
	}()
	return w
}
