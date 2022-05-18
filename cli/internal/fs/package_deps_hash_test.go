package fs

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/stretchr/testify/assert"
)

// @todo special characters
// func Test_parseGitFilename(t *testing.T) {
// 	assert.EqualValues(t, `some/path/to/a/file name`, parseGitFilename(`some/path/to/a/file name`))
// 	assert.EqualValues(t, `some/path/to/a/file name`, parseGitFilename(`some/path/to/a/file name`))
// 	assert.EqualValues(t, `some/path/to/a/file?name`, parseGitFilename(`"some/path/to/a/file?name"`))
// 	assert.EqualValues(t, `some/path/to/a/file\\name`, parseGitFilename(`"some/path/to/a/file\\\\name"`))
// 	assert.EqualValues(t, `some/path/to/a/file"name`, parseGitFilename(`"some/path/to/a/file\\"name"`))
// 	assert.EqualValues(t, `some/path/to/a/file"name`, parseGitFilename(`"some/path/to/a/file\\"name"`))
// 	assert.EqualValues(t, `some/path/to/a/file网网name`, parseGitFilename(`"some/path/to/a/file\\347\\275\\221\\347\\275\\221name"`))
// 	assert.EqualValues(t, `some/path/to/a/file\\347\\网name`, parseGitFilename(`"some/path/to/a/file\\\\347\\\\\\347\\275\\221name"`))
// 	assert.EqualValues(t, `some/path/to/a/file\\网网name`, parseGitFilename(`"some/path/to/a/file\\\\\\347\\275\\221\\347\\275\\221name"`))
// }

func Test_parseGitStatus(t *testing.T) {

	want := map[string]string{
		"turboooz.config.js":        "R",
		"package_deps_hash.go":      "??",
		"package_deps_hash_test.go": "??",
	}
	input := `
R  turbo.config.js -> turboooz.config.js
?? package_deps_hash.go
?? package_deps_hash_test.go`
	assert.EqualValues(t, want, parseGitStatus(input, ""))
}
func Test_getPackageDeps(t *testing.T) {

	want := map[string]string{
		"turboooz.config.js":        "R",
		"package_deps_hash.go":      "??",
		"package_deps_hash_test.go": "??",
	}
	input := `
R  turbo.config.js -> turboooz.config.js
?? package_deps_hash.go
?? package_deps_hash_test.go`
	assert.EqualValues(t, want, parseGitStatus(input, ""))
}

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
	hashes, err := GetHashableDeps([]string{turboPath, makefilePath, mainPath}, cliDir)
	if err != nil {
		t.Fatalf("failed to hash files: %v", err)
	}
	// Note that the paths here are platform independent, so hardcoded slashes should be fine
	expected := []string{
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
		keys := []string{}
		for key := range hashes {
			keys = append(keys, key)
		}
		t.Errorf("hashes mismatch. got %v want %v", strings.Join(keys, ", "), strings.Join(expected, ", "))
	}
}
