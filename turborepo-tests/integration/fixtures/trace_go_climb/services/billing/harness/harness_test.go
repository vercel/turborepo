package harness

import (
	"path/filepath"
	"runtime"
	"testing"

	"example.com/billing/config"
)

// repoRoot is the repository root, found from this file's own location.
func repoRoot() string {
	_, file, _, _ := runtime.Caller(0)
	return filepath.Join(filepath.Dir(file), "../../..")
}

func TestWebSettings(t *testing.T) {
	if _, err := config.Load(filepath.Join(repoRoot(), "apps", "web", "settings.json")); err != nil {
		t.Fatal(err)
	}
}
