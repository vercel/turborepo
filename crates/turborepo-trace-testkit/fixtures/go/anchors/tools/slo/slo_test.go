package slo

import (
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

// servicesDir finds the services of the repository from the location of this file.
func servicesDir(t *testing.T) string {
	t.Helper()
	_, thisFile, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatalf("cannot determine the current file")
	}
	return filepath.Join(filepath.Dir(thisFile), "..", "..", "services")
}

func TestEveryServiceHasAnOwner(t *testing.T) {
	dir := servicesDir(t)
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatalf("list %s: %v", dir, err)
	}
	for _, e := range entries {
		data, err := os.ReadFile(filepath.Join(dir, e.Name(), "owner.txt"))
		if err != nil || len(data) == 0 {
			t.Errorf("service %s has no owner: %v", e.Name(), err)
		}
	}
}
