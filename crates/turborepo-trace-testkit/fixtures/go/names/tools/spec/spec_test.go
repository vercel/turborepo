package spec

import (
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

// specFile is the order spec next to this file, found from the file's own location.
func specFile(t *testing.T) string {
	t.Helper()
	_, self, _, found := runtime.Caller(0)
	if !found {
		t.Fatal("caller unknown")
	}
	return filepath.Join(filepath.Dir(self), "specs", "order.json")
}

func specSize(t *testing.T, file string) int {
	t.Helper()
	b, err := os.ReadFile(file)
	if err != nil {
		t.Fatal(err)
	}
	return len(b)
}

func TestSpecPresent(t *testing.T) {
	if specSize(t, specFile(t)) == 0 {
		t.Fatal("empty spec")
	}
}
