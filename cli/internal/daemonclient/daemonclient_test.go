package daemonclient

import (
	"path/filepath"
	"testing"
)

func TestFormatRepoRelativeGlob(t *testing.T) {
	rawGlob := filepath.Join("some", "path:in", "repo", "**", "*.ts")
	// Note that we expect unix slashes whether or not we are on Windows
	expected := "some/path\\:in/repo/**/*.ts"

	result := formatRepoRelativeGlob(rawGlob)
	if result != expected {
		t.Errorf("formatRepoRelativeGlob(%v) got %v, want %v", rawGlob, result, expected)
	}
}
