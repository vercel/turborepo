package extract

import (
	"os"
	"path/filepath"
)

// Unpack writes the archive entries below root.
func Unpack(root string, entries map[string]string) error {
	for name, body := range entries {
		p := filepath.Join(root, name)
		if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
			return err
		}
		if err := os.WriteFile(p, []byte(body), 0o644); err != nil {
			return err
		}
	}
	return nil
}
