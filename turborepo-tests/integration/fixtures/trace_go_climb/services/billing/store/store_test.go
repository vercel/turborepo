package store

import (
	"path/filepath"
	"testing"

	"example.com/billing/config"
)

func TestAdminSettings(t *testing.T) {
	// A base below the package directory: the string climbs further than the path.
	dir := "testdata/golden"
	if _, err := config.Load(filepath.Join(dir, "../../../../../apps/admin/settings.json")); err != nil {
		t.Fatal(err)
	}
}
