package runsummary

import (
	"context"
	"errors"
	"sync"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
)

type failFirstClient struct {
	mu                 sync.Mutex
	sawFirst           bool
	additionalRequests int
}

func (f *failFirstClient) IsLinked() bool {
	return true
}

func (f *failFirstClient) request() ([]byte, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.sawFirst {
		f.additionalRequests++
		return []byte("some response"), nil
	}
	f.sawFirst = true
	return nil, errors.New("failed request")
}

func (f *failFirstClient) JSONPost(_ context.Context, _ string, _ []byte) ([]byte, error) {
	return f.request()
}

func (f *failFirstClient) JSONPatch(_ context.Context, _ string, _ []byte) ([]byte, error) {
	return f.request()
}

func TestFailToCreateRun(t *testing.T) {
	api := &failFirstClient{}

	c := newSpacesClient("my-space-id", api)
	go c.start()
	payload := &spacesRunPayload{}
	c.createRun(payload)
	exitCode := 1
	ts := &TaskSummary{
		TaskID:  "my-id",
		Task:    "task",
		Package: "package",
		Hash:    "hash",
		Execution: &TaskExecutionSummary{
			startAt:  time.Now(),
			Duration: 3 * time.Second,
			exitCode: &exitCode,
		},
	}
	c.postTask(ts)
	c.postTask(ts)
	c.postTask(ts)
	c.Close()

	assert.True(t, api.sawFirst)
	assert.Equal(t, api.additionalRequests, 0)
}
