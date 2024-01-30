package daemonclient

import (
	"path/filepath"
	"runtime"
	"testing"
)

func TestFormatRepoRelativeGlob(t *testing.T) {
	rawGlob := filepath.Join("some", ".turbo", "turbo-foo:bar.log")
	// Note that we expect unix slashes whether or not we are on Windows
	var expected string
	if runtime.GOOS == "windows" {
		expected = "some/.turbo/turbo-foo"
	} else {
		expected = "some/.turbo/turbo-foo\\:bar.log"
	}

	result := formatRepoRelativeGlob(rawGlob)
	if result != expected {
		t.Errorf("formatRepoRelativeGlob(%v) got %v, want %v", rawGlob, result, expected)
	}
}
