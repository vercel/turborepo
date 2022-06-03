package signals

import (
	"os"
	"os/signal"
	"sync"
	"syscall"
)

type Watcher struct {
	doneCh  chan struct{}
	closed  bool
	mu      sync.Mutex
	closers []func()
}

func (w *Watcher) AddOnClose(closer func()) {
	w.mu.Lock()
	defer w.mu.Unlock()
	w.closers = append(w.closers, closer)
}

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

func (w *Watcher) Done() <-chan struct{} {
	return w.doneCh
}

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

// func watchSignals(onClose func()) <-chan struct{} {
// 	doneCh := make(chan struct{})
// 	signalCh := make(chan os.Signal, 1)
// 	signal.Notify(signalCh, os.Interrupt, syscall.SIGTERM, syscall.SIGQUIT)
// 	go func() {
// 		<-signalCh
// 		onClose()
// 		close(doneCh)
// 	}()
// 	return doneCh
// }
