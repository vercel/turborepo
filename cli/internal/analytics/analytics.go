package analytics

import (
	"context"
	"time"

	"github.com/google/uuid"
	"github.com/vercel/turborepo/cli/internal/util"
)

type Events struct {
	SessionID uuid.UUID      `json:"sessionId"`
	Payloads  []EventPayload `json:"events"`
}
type EventPayload = interface{}

type Recorder interface {
	LogEvent(payload EventPayload)
}

type Client interface {
	Recorder
	Close()
}

type Sink interface {
	RecordAnalyticsEvents(events *Events) error
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
	sessionID     uuid.UUID
	sink          Sink
}

const bufferThreshold = 10
const eventTimeout = 200 * time.Millisecond
const noTimeout = 24 * time.Hour

func newWorker(ctx context.Context, ch <-chan EventPayload, sink Sink) *worker {
	buffer := []EventPayload{}
	sessionID := uuid.New()
	w := &worker{
		buffer:        buffer,
		ch:            ch,
		ctx:           ctx,
		doneSemaphore: util.NewSemaphore(1),
		sessionID:     sessionID,
		sink:          sink,
	}
	w.doneSemaphore.Acquire()
	go w.analyticsClient()
	return w
}

func NewClient(parent context.Context, sink Sink) Client {
	ch := make(chan EventPayload)
	ctx, cancel := context.WithCancel(parent)
	// creates and starts the worker
	worker := newWorker(ctx, ch, sink)
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
		err := w.sendEvents(w.buffer)
		if err != nil {
			// TODO: log error? error count and stop sending?
		}
		w.buffer = []EventPayload{}
	}
}

func (w *worker) sendEvents(payloads []EventPayload) error {
	events := &Events{
		SessionID: w.sessionID,
		Payloads:  payloads,
	}
	return w.sink.RecordAnalyticsEvents(events)
}
