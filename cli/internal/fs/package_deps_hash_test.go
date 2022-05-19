package fs

import (
	"os"
	"path/filepath"
	"testing"
)

func Test_GetHashableDeps(t *testing.T) {
	cwd, err := os.Getwd()
	if err != nil {
		t.Fatalf("failed to get cwd %v", err)
	}
	cliDir, err := filepath.Abs(filepath.Join(cwd, "..", ".."))
	if err != nil {
		t.Fatalf("failed to get cli dir: %v", err)
	}
	if filepath.Base(cliDir) != "cli" {
		t.Fatalf("did not find cli dir, found %v", cliDir)
	}
	turboPath := filepath.Join(cliDir, "..", "turbo.json")
	makefilePath := filepath.Join(cliDir, "Makefile")
	mainPath := filepath.Join(cliDir, "cmd", "turbo", "main.go")
	hashes, err := GetHashableDeps(UnsafeToAbsolutePath(cliDir), []string{turboPath, makefilePath, mainPath})
	if err != nil {
		t.Fatalf("failed to hash files: %v", err)
	}
	// Note that the paths here are platform independent, so hardcoded slashes should be fine
	expected := []RepoRelativeUnixPath{
		"../turbo.json",
		"Makefile",
		"cmd/turbo/main.go",
	}
	for _, key := range expected {
		if _, ok := hashes[key]; !ok {
			t.Errorf("hashes missing %v", key)
		}
	}
	if len(hashes) != len(expected) {
		keys := []RepoRelativeUnixPath{}
		for key := range hashes {
			keys = append(keys, key)
		}
		t.Errorf("hashes mismatch. got %v want %v", keys, expected)
	}
}
