package analytics

import (
	"context"
	"sync"
	"testing"
	"time"
)

type dummySink struct {
	events []*Events
	err    error
	mu     sync.Mutex
}

type evt struct {
	I int
}

func newDummySink() *dummySink {
	return &dummySink{
		events: []*Events{},
	}
}

func (d *dummySink) RecordAnalyticsEvents(events *Events) error {
	d.mu.Lock()
	defer d.mu.Unlock()
	// Make a copy in case a test is holding a copy too
	eventsCopy := make([]*Events, len(d.events))
	copy(eventsCopy, d.events)
	d.events = append(eventsCopy, events)
	return d.err
}

func (d *dummySink) Events() []*Events {
	d.mu.Lock()
	defer d.mu.Unlock()
	return d.events
}

func Test_batching(t *testing.T) {
	d := newDummySink()
	ctx := context.Background()
	c := NewClient(ctx, d)
	for i := 0; i < 2; i++ {
		c.LogEvent(&evt{i})
	}
	found := d.Events()
	if len(found) != 0 {
		t.Errorf("got %v events, want 0 due to batching", len(found))
	}
	// Should timeout
	<-time.After(210 * time.Millisecond)
	found = d.Events()
	if len(found) != 1 {
		t.Errorf("got %v, want 1 batch to have been flushed", len(found))
	}
	payloads := found[0].Payloads
	if len(payloads) != 2 {
		t.Errorf("got %v, want 2 payloads to have been flushed", len(payloads))
	}
}

func Test_batchingAcrossTwoBatches(t *testing.T) {
	d := newDummySink()
	ctx := context.Background()
	c := NewClient(ctx, d)
	for i := 0; i < 12; i++ {
		c.LogEvent(&evt{i})
	}
	// Short timeout to allow for first batch
	<-time.After(10 * time.Millisecond)
	found := d.Events()
	if len(found) != 1 {
		t.Errorf("got %v, want 1 batch to have been flushed", len(found))
	}
	payloads := found[0].Payloads
	if len(payloads) != 10 {
		t.Errorf("got %v, want 10 payloads to have been flushed", len(payloads))
	}
	// Should timeout second batch
	<-time.After(210 * time.Millisecond)
	found = d.Events()
	if len(found) != 2 {
		t.Errorf("got %v, want 2 batches to have been flushed", len(found))
	}
	payloads = found[1].Payloads
	if len(payloads) != 2 {
		t.Errorf("got %v, want 2 payloads to have been flushed", len(payloads))
	}
}

func Test_closing(t *testing.T) {
	d := newDummySink()
	ctx := context.Background()
	c := NewClient(ctx, d)
	for i := 0; i < 2; i++ {
		c.LogEvent(&evt{i})
	}
	found := d.Events()
	if len(found) != 0 {
		t.Errorf("got %v events, want 0 due to batching", len(found))
	}
	c.Close()
	found = d.Events()
	if len(found) != 1 {
		t.Errorf("got %v, want 1 batch to have been flushed", len(found))
	}
	payloads := found[0].Payloads
	if len(payloads) != 2 {
		t.Errorf("got %v, want 2 payloads to have been flushed", len(payloads))
	}
}

func Test_closingByContext(t *testing.T) {
	d := newDummySink()
	ctx, cancel := context.WithCancel(context.Background())
	c := NewClient(ctx, d)
	for i := 0; i < 2; i++ {
		c.LogEvent(&evt{i})
	}
	found := d.Events()
	if len(found) != 0 {
		t.Errorf("got %v events, want 0 due to batching", len(found))
	}
	cancel()
	<-time.After(10 * time.Millisecond)
	found = d.Events()
	if len(found) != 1 {
		t.Errorf("got %v, want 1 batch to have been flushed", len(found))
	}
	payloads := found[0].Payloads
	if len(payloads) != 2 {
		t.Errorf("got %v, want 2 payloads to have been flushed", len(payloads))
	}
}
