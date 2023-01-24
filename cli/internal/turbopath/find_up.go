package turbopath

import (
	"io/ioutil"
	"os"
	"path/filepath"
)

type readDir func(string) ([]os.FileInfo, error)

var defaultReadDir readDir = ioutil.ReadDir

func hasFile(name, dir string, readdir readDir) (bool, error) {
	files, err := readdir(dir)

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

func findupFrom(name, dir string, readdir readDir) (string, error) {
	for {
		found, err := hasFile(name, dir, readdir)

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

// Recursively find a file by walking up parents in the file tree
// starting from a specific directory.
func FindupFrom(name, dir string) (string, error) {
	return findupFrom(name, dir, defaultReadDir)
}
