package audit

import (
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

// svcRoot is the services directory, two levels above this file.
func svcRoot(t *testing.T) string {
	t.Helper()
	_, self, _, found := runtime.Caller(0)
	if !found {
		t.Fatal("caller unknown")
	}
	return filepath.Join(filepath.Dir(self), "..", "..", "services")
}

func TestOwnersFiles(t *testing.T) {
	root := svcRoot(t)
	list, err := os.ReadDir(root)
	if err != nil {
		t.Fatal(err)
	}
	for _, svc := range list {
		owner, err := os.ReadFile(filepath.Join(root, svc.Name(), "owner.txt"))
		if err != nil {
			t.Errorf("%s: %v", svc.Name(), err)
		} else if len(owner) == 0 {
			t.Errorf("%s: empty owner.txt", svc.Name())
		}
	}
}
