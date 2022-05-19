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

	repoRoot, err := filepath.Abs(filepath.Join(cwd, "..", "..", ".."))
	if err != nil {
		t.Fatalf("failed to get repo root dir: %v", err)
	}

	turboPath := filepath.Join("turbo.json")
	makefilePath := filepath.Join("cli", "Makefile")
	mainPath := filepath.Join("cli", "cmd", "turbo", "main.go")
	hashes, err := GetHashableDeps(UnsafeToAbsolutePath(repoRoot), []string{turboPath, makefilePath, mainPath})

	if err != nil {
		t.Fatalf("failed to hash files: %v", err)
	}
	// Note that the paths here are platform independent, so hardcoded slashes should be fine
	expected := []RepoRelativeUnixPath{
		"turbo.json",
		"cli/Makefile",
		"cli/cmd/turbo/main.go",
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
