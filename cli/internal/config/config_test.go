package config

import (
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/ui"
)

func TestEnvironmentToken(t *testing.T) {
	expectedToken := "my-token"
	err := os.Setenv("TURBO_TOKEN", expectedToken)
	if err != nil {
		t.Fatalf("setenv %v", err)
	}

	terminal := ui.Default()
	cfg, err := ParseAndValidate([]string{"run", "build"}, terminal, "my-version")
	if err != nil {
		t.Fatalf("failed to parse config %v", err)
	}
	assert.Equal(t, cfg.Token, expectedToken)
}

func TestSelectCwd(t *testing.T) {
	defaultCwd, err := fs.GetCwd()
	if err != nil {
		t.Errorf("failed to get cwd: %v", err)
	}

	tempDir, err := os.MkdirTemp("", "turbo-test")
	if err != nil {
		t.Fatalf("MkdirTemp %v", err)
	}
	resolvedTempDir, err := filepath.EvalSymlinks(tempDir)
	if err != nil {
		t.Fatalf("EvalSymlinks %v", err)
	}

	cases := []struct {
		Name      string
		InputArgs []string
		Expected  fs.AbsolutePath
	}{
		{
			Name:      "default",
			InputArgs: []string{"foo"},
			Expected:  defaultCwd,
		},
		{
			Name:      "choose command-line flag cwd",
			InputArgs: []string{"foo", "--cwd=" + tempDir},
			Expected:  fs.UnsafeToAbsolutePath(resolvedTempDir),
		},
		{
			Name:      "ignore other flags not cwd",
			InputArgs: []string{"foo", "--ignore-this-1", "--cwd=" + tempDir, "--ignore-this=2"},
			Expected:  fs.UnsafeToAbsolutePath(resolvedTempDir),
		},
		{
			Name:      "ignore args after pass through",
			InputArgs: []string{"foo", "--", "--cwd=zop"},
			Expected:  defaultCwd,
		},
	}

	for i, tc := range cases {
		t.Run(fmt.Sprintf("%d-%s", i, tc.Name), func(t *testing.T) {
			actual, err := selectCwd(tc.InputArgs)
			if err != nil {
				t.Fatalf("invalid parse: %#v", err)
			}
			assert.EqualValues(t, tc.Expected, actual)
		})
	}

}
