package cmdutil

import (
	"os"
	"testing"
	"time"

	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbostate"
	"gotest.tools/v3/assert"
)

func TestTokenEnvVar(t *testing.T) {
	// Set up an empty config so we're just testing environment variables
	userConfigPath := fs.AbsoluteSystemPathFromUpstream(t.TempDir()).UntypedJoin("turborepo", "config.json")
	expectedPrefix := "my-token"
	vars := []string{"TURBO_TOKEN", "VERCEL_ARTIFACTS_TOKEN"}
	for _, v := range vars {
		t.Run(v, func(t *testing.T) {
			t.Cleanup(func() {
				_ = os.Unsetenv(v)
			})
			args := &turbostate.ParsedArgsFromRust{
				CWD: "",
			}
			h := NewHelper("test-version", args)
			h.UserConfigPath = userConfigPath

			expectedToken := expectedPrefix + v
			err := os.Setenv(v, expectedToken)
			if err != nil {
				t.Fatalf("setenv %v", err)
			}

			base, err := h.GetCmdBase(args)
			if err != nil {
				t.Fatalf("failed to get command base %v", err)
			}
			assert.Equal(t, base.RemoteConfig.Token, expectedToken)
		})
	}
}

func TestRemoteCacheTimeoutEnvVar(t *testing.T) {
	key := "TURBO_REMOTE_CACHE_TIMEOUT"
	expectedTimeout := "600"
	t.Run(key, func(t *testing.T) {
		t.Cleanup(func() {
			_ = os.Unsetenv(key)
		})
		args := &turbostate.ParsedArgsFromRust{
			CWD: "",
		}
		h := NewHelper("test-version", args)

		err := os.Setenv(key, expectedTimeout)
		if err != nil {
			t.Fatalf("setenv %v", err)
		}

		base, err := h.GetCmdBase(args)
		if err != nil {
			t.Fatalf("failed to get command base %v", err)
		}
		assert.Equal(t, base.APIClient.HTTPClient.HTTPClient.Timeout, time.Duration(600)*time.Second)
	})
}

func TestRemoteCacheTimeoutFlag(t *testing.T) {
	args := &turbostate.ParsedArgsFromRust{
		CWD:                "",
		RemoteCacheTimeout: 599,
	}
	h := NewHelper("test-version", args)

	base, err := h.GetCmdBase(args)
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
		args := &turbostate.ParsedArgsFromRust{
			CWD:                "",
			RemoteCacheTimeout: 1,
		}
		h := NewHelper("test-version", args)

		err := os.Setenv(key, value)
		if err != nil {
			t.Fatalf("setenv %v", err)
		}

		base, err := h.GetCmdBase(args)
		if err != nil {
			t.Fatalf("failed to get command base %v", err)
		}
		assert.Equal(t, base.APIClient.HTTPClient.HTTPClient.Timeout, time.Duration(1)*time.Second)
	})
}
