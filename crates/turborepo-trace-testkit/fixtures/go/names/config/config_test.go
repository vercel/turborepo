package config

import (
	"os"
	"path/filepath"
	"testing"
)

func TestLoad(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "settings.yaml")
	if err := os.WriteFile(path, []byte("mode: test\n"), 0o600); err != nil {
		t.Fatalf("write settings.yaml: %v", err)
	}
	got, err := Load(path)
	if err != nil || len(got) == 0 {
		t.Fatalf("load: %v", err)
	}
}
