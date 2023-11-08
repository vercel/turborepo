package cmdutil

import (
	"os"
	"testing"
	"time"

	"github.com/vercel/turbo/cli/internal/turbostate"
	"gotest.tools/v3/assert"
)

func TestRemoteCacheTimeoutFlag(t *testing.T) {
	args := turbostate.ParsedArgsFromRust{
		CWD: "",
	}

	executionState := turbostate.ExecutionState{
		APIClientConfig: turbostate.APIClientConfig{
			Timeout: 599,
		},
		CLIArgs: args,
	}

	h := NewHelper("test-version", &args)

	base, err := h.GetCmdBase(&executionState)
	if err != nil {
		t.Fatalf("failed to get command base %v", err)
	}

	assert.Equal(t, base.APIClient.HTTPClient.HTTPClient.Timeout, time.Duration(599)*time.Second)
}

func TestRemoteCacheTimeoutPrimacy(t *testing.T) {
	key := "TURBO_REMOTE_CACHE_TIMEOUT"
	value := "2"

	t.Run(key, func(t *testing.T) {
		t.Cleanup(func() {
			_ = os.Unsetenv(key)
		})
		args := turbostate.ParsedArgsFromRust{
			CWD: "",
		}
		executionState := turbostate.ExecutionState{
			APIClientConfig: turbostate.APIClientConfig{
				Timeout: 1,
			},
			CLIArgs: args,
		}
		h := NewHelper("test-version", &args)

		err := os.Setenv(key, value)
		if err != nil {
			t.Fatalf("setenv %v", err)
		}

		base, err := h.GetCmdBase(&executionState)
		if err != nil {
			t.Fatalf("failed to get command base %v", err)
		}
		assert.Equal(t, base.APIClient.HTTPClient.HTTPClient.Timeout, time.Duration(1)*time.Second)
	})
}
