package analytics

import (
	"context"
	"sync"
	"testing"
	"time"

	"github.com/hashicorp/go-hclog"
)

type dummySink struct {
	events []*Events
	err    error
	mu     sync.Mutex
	ch     chan struct{}
}

type evt struct {
	I int
}

func newDummySink() *dummySink {
	return &dummySink{
		events: []*Events{},
		ch:     make(chan struct{}, 1),
	}
}

func (d *dummySink) RecordAnalyticsEvents(events Events) error {
	d.mu.Lock()
	defer d.mu.Unlock()
	// Make a copy in case a test is holding a copy too
	eventsCopy := make([]*Events, len(d.events))
	copy(eventsCopy, d.events)
	d.events = append(eventsCopy, &events)
	d.ch <- struct{}{}
	return d.err
}

func (d *dummySink) Events() []*Events {
	d.mu.Lock()
	defer d.mu.Unlock()
	return d.events
}

func (d *dummySink) ExpectImmediateMessage(t *testing.T) {
	select {
	case <-time.After(150 * time.Millisecond):
		t.Errorf("expected to not wait out the flush timeout")
	case <-d.ch:
	}
}

func (d *dummySink) ExpectTimeoutThenMessage(t *testing.T) {
	select {
	case <-d.ch:
		t.Errorf("Expected to wait out the flush timeout")
	case <-time.After(150 * time.Millisecond):
	}
	<-d.ch
}

func Test_batching(t *testing.T) {
	d := newDummySink()
	ctx := context.Background()
	c := NewClient(ctx, d, hclog.Default())
	for i := 0; i < 2; i++ {
		c.LogEvent(&evt{i})
	}
	found := d.Events()
	if len(found) != 0 {
		t.Errorf("got %v events, want 0 due to batching", len(found))
	}
	// Should timeout
	d.ExpectTimeoutThenMessage(t)
	found = d.Events()
	if len(found) != 1 {
		t.Errorf("got %v, want 1 batch to have been flushed", len(found))
	}
	payloads := *found[0]
	if len(payloads) != 2 {
		t.Errorf("got %v, want 2 payloads to have been flushed", len(payloads))
	}
}

func Test_batchingAcrossTwoBatches(t *testing.T) {
	d := newDummySink()
	ctx := context.Background()
	c := NewClient(ctx, d, hclog.Default())
	for i := 0; i < 12; i++ {
		c.LogEvent(&evt{i})
	}
	// We sent more than the batch size, expect a message immediately
	d.ExpectImmediateMessage(t)
	found := d.Events()
	if len(found) != 1 {
		t.Errorf("got %v, want 1 batch to have been flushed", len(found))
	}
	payloads := *found[0]
	if len(payloads) != 10 {
		t.Errorf("got %v, want 10 payloads to have been flushed", len(payloads))
	}
	// Should timeout second batch
	d.ExpectTimeoutThenMessage(t)
	found = d.Events()
	if len(found) != 2 {
		t.Errorf("got %v, want 2 batches to have been flushed", len(found))
	}
	payloads = *found[1]
	if len(payloads) != 2 {
		t.Errorf("got %v, want 2 payloads to have been flushed", len(payloads))
	}
}

func Test_closing(t *testing.T) {
	d := newDummySink()
	ctx := context.Background()
	c := NewClient(ctx, d, hclog.Default())
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
	payloads := *found[0]
	if len(payloads) != 2 {
		t.Errorf("got %v, want 2 payloads to have been flushed", len(payloads))
	}
}

func Test_closingByContext(t *testing.T) {
	d := newDummySink()
	ctx, cancel := context.WithCancel(context.Background())
	c := NewClient(ctx, d, hclog.Default())
	for i := 0; i < 2; i++ {
		c.LogEvent(&evt{i})
	}
	found := d.Events()
	if len(found) != 0 {
		t.Errorf("got %v events, want 0 due to batching", len(found))
	}
	cancel()
	d.ExpectImmediateMessage(t)
	found = d.Events()
	if len(found) != 1 {
		t.Errorf("got %v, want 1 batch to have been flushed", len(found))
	}
	payloads := *found[0]
	if len(payloads) != 2 {
		t.Errorf("got %v, want 2 payloads to have been flushed", len(payloads))
	}
}

func Test_addSessionId(t *testing.T) {
	events := []struct {
		Foo string `mapstructure:"foo"`
	}{
		{
			Foo: "foo1",
		},
		{
			Foo: "foo2",
		},
	}
	arr := make([]interface{}, len(events))
	for i, event := range events {
		arr[i] = event
	}
	sessionID := "my-uuid"
	output, err := addSessionID(sessionID, arr)
	if err != nil {
		t.Errorf("failed to encode analytics events: %v", err)
	}
	if len(output) != 2 {
		t.Errorf("len output got %v, want 2", len(output))
	}
	if output[0]["foo"] != "foo1" {
		t.Errorf("first event foo got %v, want foo1", output[0]["foo"])
	}
	for i, event := range output {
		if event["sessionId"] != "my-uuid" {
			t.Errorf("event %v sessionId got %v, want %v", i, event["sessionId"], sessionID)
		}
	}
}
