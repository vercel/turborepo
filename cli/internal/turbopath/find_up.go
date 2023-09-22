package turbopath

import (
	"os"
	"path/filepath"
)

func hasFile(name, dir string) (bool, error) {
	files, err := os.ReadDir(dir)

	if err != nil {
		return false, err
	}

	for _, f := range files {
		if name == f.Name() {
			return true, nil
		}
	}

	return false, nil
}

func findupFrom(name, dir string) (string, error) {
	for {
		found, err := hasFile(name, dir)

		if err != nil {
			return "", err
		}

		if found {
			return filepath.Join(dir, name), nil
		}

		parent := filepath.Dir(dir)

		if parent == dir {
			return "", nil
		}

		dir = parent
	}
}

// FindupFrom Recursively finds a file by walking up parents in the file tree
// starting from a specific directory.
func FindupFrom(name, dir string) (string, error) {
	return findupFrom(name, dir)
}
