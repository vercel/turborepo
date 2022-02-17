package analytics

import (
	"context"
	"time"

	"github.com/vercel/turborepo/cli/internal/util"
)

type EventPayload = interface{}

type Recorder interface {
	LogEvent(payload EventPayload)
}

type Client interface {
	Recorder
	Close()
}

type client struct {
	ch     chan<- EventPayload
	cancel func()

	worker *worker
}

type worker struct {
	buffer        []EventPayload
	ch            <-chan EventPayload
	ctx           context.Context
	doneSemaphore util.Semaphore
	sessionID     string
}

const bufferThreshold = 10
const eventTimeout = 200 * time.Millisecond
const noTimeout = 24 * time.Hour

func newWorker(ctx context.Context, ch <-chan EventPayload) *worker {
	buffer := make([]EventPayload, bufferThreshold)
	sessionID := "TODO: uuid"
	w := &worker{
		buffer:        buffer,
		ch:            ch,
		ctx:           ctx,
		doneSemaphore: util.NewSemaphore(1),
		sessionID:     sessionID,
	}
	w.doneSemaphore.Acquire()
	go w.analyticsClient()
	return w
}

func NewClient(parent context.Context) Client {
	ch := make(chan EventPayload)
	ctx, cancel := context.WithCancel(parent)
	// creates and starts the worker
	worker := newWorker(ctx, ch)
	s := &client{
		ch:     ch,
		cancel: cancel,
		worker: worker,
	}
	return s
}

func (s *client) LogEvent(event EventPayload) {
	s.ch <- event
}

func (s *client) Close() {
	s.cancel()
	s.worker.Wait()
}

func (w *worker) Wait() {
	w.doneSemaphore.Acquire()
}

func (w *worker) analyticsClient() {
	timeout := time.After(noTimeout)
	for {
		select {
		case e := <-w.ch:
			w.buffer = append(w.buffer, e)
			if len(w.buffer) == bufferThreshold {
				w.flush()
				timeout = time.After(noTimeout)
			} else {
				timeout = time.After(eventTimeout)
			}
		case <-timeout:
			w.flush()
			timeout = time.After(noTimeout)
		case <-w.ctx.Done():
			w.flush()
			w.doneSemaphore.Release()
			return
		}
	}
}

func (w *worker) flush() {
	if len(w.buffer) > 0 {
		err := sendEvents(w.buffer)
		if err != nil {
			// TODO: log error? error count and stop sending?
		}
		w.buffer = make([]EventPayload, bufferThreshold)
	}
}

func sendEvents(events []EventPayload) error {
	// TODO: implement
	return nil
}
