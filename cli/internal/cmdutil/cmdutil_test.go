package cmdutil

import (
	"os"
	"testing"
	"time"

	"github.com/vercel/turbo/cli/internal/config"

	"github.com/spf13/pflag"
	"github.com/vercel/turbo/cli/internal/fs"
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
			flags := pflag.NewFlagSet("test-flags", pflag.ContinueOnError)
			h := NewHelper("test-version")
			h.AddFlags(flags)
			h.UserConfigPath = userConfigPath

			expectedToken := expectedPrefix + v
			err := os.Setenv(v, expectedToken)
			if err != nil {
				t.Fatalf("setenv %v", err)
			}

			base, err := h.GetCmdBase(config.FlagSet{FlagSet: flags})
			if err != nil {
				t.Fatalf("failed to get command base %v", err)
			}
			assert.Equal(t, base.RemoteConfig.Token, expectedToken)
		})
	}
}

func TestTuroRemoteCacheTimeoutEnvVar(t *testing.T) {
	// Set up an empty config so we're just testing environment variables
	userConfigPath := fs.AbsoluteSystemPathFromUpstream(t.TempDir()).UntypedJoin("turborepo", "config.json")
	expectedTimeout := "600"

	t.Cleanup(func() {
		_ = os.Unsetenv("TURBO_REMOTE_CACHE_TIMEOUT")
	})

	flags := pflag.NewFlagSet("test-flags", pflag.ContinueOnError)
	h := NewHelper("test-version")
	h.AddFlags(flags)
	h.UserConfigPath = userConfigPath

	err := os.Setenv("TURBO_REMOTE_CACHE_TIMEOUT", expectedTimeout)
	if err != nil {
		t.Fatalf("setenv %v", err)
	}

	base, err := h.GetCmdBase(flags)
	if err != nil {
		t.Fatalf("failed to get command base %v", err)
	}

	assert.Equal(t, base.APIClient.HttpClient.HTTPClient.Timeout, time.Duration(600)*time.Second)
}

func TestRemoteCacheTimeoutFlag(t *testing.T) {
	// Set up an empty config so we're just testing environment variables
	userConfigPath := fs.AbsoluteSystemPathFromUpstream(t.TempDir()).UntypedJoin("turborepo", "config.json")
	expectedTimeout := "600"

	flags := pflag.NewFlagSet("test-flags", pflag.ContinueOnError)
	h := NewHelper("test-version")
	h.AddFlags(flags)
	h.UserConfigPath = userConfigPath

	assert.NilError(t, flags.Set("remote-cache-timeout", expectedTimeout), "flags.Set")

	base, err := h.GetCmdBase(flags)
	if err != nil {
		t.Fatalf("failed to get command base %v", err)
	}

	assert.Equal(t, base.APIClient.HttpClient.HTTPClient.Timeout, time.Duration(600)*time.Second)
}
