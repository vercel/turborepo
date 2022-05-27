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

	turboPath := StringToSystemPath(filepath.Join(cliDir, "..", "turbo.json"))
	makefilePath := StringToSystemPath(filepath.Join(cliDir, "Makefile"))
	mainPath := StringToSystemPath(filepath.Join(cliDir, "cmd", "turbo", "main.go"))

	hashes, err := GetHashableDeps(UnsafeToAbsolutePath(cliDir), []FilePathInterface{turboPath, makefilePath, mainPath})
	if err != nil {
		t.Fatalf("failed to hash files: %v", err)
	}
	// Note that the paths here are platform independent, so hardcoded slashes should be fine
	expected := []RelativeUnixPath{
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
		keys := []RelativeUnixPath{}
		for key := range hashes {
			keys = append(keys, key)
		}
		t.Errorf("hashes mismatch. got %v want %v", keys, expected)
	}
}
