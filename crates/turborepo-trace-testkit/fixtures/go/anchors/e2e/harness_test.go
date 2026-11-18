package e2e

import (
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

// harness is what the tests of this package share: the directory of the service under test.
type harness struct {
	apiDir string
}

// newHarness finds the repository from the location of this file (e2e → repository), so
// that the tests run wherever `go test` is started.
func newHarness(t *testing.T) *harness {
	t.Helper()
	_, file, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("cannot determine the current file")
	}
	root := filepath.Clean(filepath.Join(filepath.Dir(file), ".."))
	if real, err := filepath.EvalSymlinks(root); err == nil {
		root = real
	}
	// A child process that runs at the root reads nothing for this process.
	cmd := exec.Command("sh", "-c", "pwd")
	cmd.Dir = root
	out, err := cmd.Output()
	if err != nil {
		t.Fatalf("pwd in %s: %v", root, err)
	}
	if strings.TrimSpace(string(out)) != root {
		t.Logf("pwd says %s, the harness %s", out, root)
	}
	return &harness{apiDir: filepath.Join(root, "services", "svc-a")}
}

// read reads a file of the service.
func (h *harness) read(t *testing.T, name string) []byte {
	t.Helper()
	data, err := os.ReadFile(filepath.Join(h.apiDir, name))
	if err != nil {
		t.Fatalf("read %s: %v", name, err)
	}
	return data
}
