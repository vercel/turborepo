package e2e

import (
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

// harness holds what the tests of this package share: the service they look at.
type harness struct {
	svcDir string
}

// newHarness locates the repository one directory above this file, wherever `go test`
// runs.
func newHarness(t *testing.T) *harness {
	t.Helper()
	_, file, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("no caller information")
	}
	root := filepath.Clean(filepath.Join(filepath.Dir(file), ".."))
	if resolved, err := filepath.EvalSymlinks(root); err == nil {
		root = resolved
	}
	// A child process started at the root reads nothing on behalf of this one.
	cmd := exec.Command("sh", "-c", "pwd")
	cmd.Dir = root
	out, err := cmd.Output()
	if err != nil {
		t.Fatalf("sh in %s: %v", root, err)
	}
	if got := strings.TrimSpace(string(out)); got != root {
		t.Logf("child at %s, harness at %s", got, root)
	}
	return &harness{svcDir: filepath.Join(root, "services", "svc-a")}
}

// read returns a file of the service.
func (h *harness) read(t *testing.T, name string) []byte {
	t.Helper()
	b, err := os.ReadFile(filepath.Join(h.svcDir, name))
	if err != nil {
		t.Fatal(err)
	}
	return b
}
