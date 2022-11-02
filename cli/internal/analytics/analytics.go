package analytics

import (
	"context"
	"sync"
	"time"

	"github.com/google/uuid"
	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/mapstructure"
	"github.com/vercel/turbo/cli/internal/util"
)

type Events = []map[string]interface{}

type EventPayload = interface{}

type Recorder interface {
	LogEvent(payload EventPayload)
}

type Client interface {
	Recorder
	Close()
	CloseWithTimeout(timeout time.Duration)
}

type Sink interface {
	RecordAnalyticsEvents(events Events) error
}

type nullSink struct{}

func (n *nullSink) RecordAnalyticsEvents(events Events) error {
	return nil
}

// NullSink is an analytics sink to use in the event that we don't want to send
// analytics
var NullSink = &nullSink{}

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
	wg            sync.WaitGroup
	logger        hclog.Logger
}

const bufferThreshold = 10
const eventTimeout = 200 * time.Millisecond
const noTimeout = 24 * time.Hour

func newWorker(ctx context.Context, ch <-chan EventPayload, sink Sink, logger hclog.Logger) *worker {
	buffer := []EventPayload{}
	sessionID := uuid.New()
	w := &worker{
		buffer:        buffer,
		ch:            ch,
		ctx:           ctx,
		doneSemaphore: util.NewSemaphore(1),
		sessionID:     sessionID,
		sink:          sink,
		logger:        logger,
	}
	w.doneSemaphore.Acquire()
	go w.analyticsClient()
	return w
}

func NewClient(parent context.Context, sink Sink, logger hclog.Logger) Client {
	ch := make(chan EventPayload)
	ctx, cancel := context.WithCancel(parent)
	// creates and starts the worker
	worker := newWorker(ctx, ch, sink, logger)
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

func (s *client) CloseWithTimeout(timeout time.Duration) {
	ch := make(chan struct{})
	go func() {
		s.Close()
		close(ch)
	}()
	select {
	case <-ch:
	case <-time.After(timeout):
	}
}

func (w *worker) Wait() {
	w.doneSemaphore.Acquire()
	w.wg.Wait()
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
		w.sendEvents(w.buffer)
		w.buffer = []EventPayload{}
	}
}

func (w *worker) sendEvents(events []EventPayload) {
	w.wg.Add(1)
	go func() {
		payload, err := addSessionID(w.sessionID.String(), events)
		if err != nil {
			w.logger.Debug("failed to encode cache usage analytics", "error", err)
		}
		err = w.sink.RecordAnalyticsEvents(payload)
		if err != nil {
			w.logger.Debug("failed to record cache usage analytics", "error", err)
		}
		w.wg.Done()
	}()
}

func addSessionID(sessionID string, events []EventPayload) (Events, error) {
	eventMaps := []map[string]interface{}{}
	err := mapstructure.Decode(events, &eventMaps)
	if err != nil {
		return nil, err
	}
	for _, event := range eventMaps {
		event["sessionId"] = sessionID
	}
	return eventMaps, nil
}
