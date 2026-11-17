// Package config reads configuration files.
package config

import "os"

// Load reads the file at path.
func Load(path string) ([]byte, error) {
	return os.ReadFile(path)
}
