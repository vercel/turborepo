package fs

import (
	"strings"
	"testing"

	"github.com/stretchr/testify/assert"
)

func Test_parseGitLsTree(t *testing.T) {
	str := strings.TrimSpace(`
	100644 blob 7d10c39d8d500db5d7dc2040016a4678a1297f2e    fs.go
100644 blob 96b98aca484a5f2775aa8fde07cfe5396a17693e    hash.go
100644 blob b9fde9650a6f1cd86eab69e8442a85d89b1e0455    hash_test.go
100644 blob e69de29bb2d1d6434b8b29ae775ad8c2e48c5391    test_data/.test
100644 blob c7c5d4814cf152aa7b7b65f338bcb05d9d70402c    test_data/test.txt
100644 blob e69de29bb2d1d6434b8b29ae775ad8c2e48c5391    test_data/test_subfolder++/test.txt
100644 blob e69de29bb2d1d6434b8b29ae775ad8c2e48c5391    test_data/test_subfolder1/a.txt
100644 blob e69de29bb2d1d6434b8b29ae775ad8c2e48c5391    test_data/test_subfolder1/sub_sub_folder/b.txt
100644 blob e69de29bb2d1d6434b8b29ae775ad8c2e48c5391    test_data/test_subfolder3/Zest.py
100644 blob e69de29bb2d1d6434b8b29ae775ad8c2e48c5391    test_data/test_subfolder3/best.py
100644 blob e69de29bb2d1d6434b8b29ae775ad8c2e48c5391    test_data/test_subfolder3/test.py
100644 blob e69de29bb2d1d6434b8b29ae775ad8c2e48c5391    test_data/test_subfolder4/TEST_BUILD
100644 blob e69de29bb2d1d6434b8b29ae775ad8c2e48c5391    test_data/test_subfolder4/test.py
100644 blob 8fd7339e6e8f7d203e61b7774fdef7692eb9c723    walk.go
	`)
	b1 := parseGitLsTree(str)
	expected := map[string]string{
		"fs.go":                               "7d10c39d8d500db5d7dc2040016a4678a1297f2e",
		"hash.go":                             "96b98aca484a5f2775aa8fde07cfe5396a17693e",
		"hash_test.go":                        "b9fde9650a6f1cd86eab69e8442a85d89b1e0455",
		"test_data/.test":                     "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
		"test_data/test.txt":                  "c7c5d4814cf152aa7b7b65f338bcb05d9d70402c",
		"test_data/test_subfolder++/test.txt": "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
		"test_data/test_subfolder1/a.txt":     "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
		"test_data/test_subfolder1/sub_sub_folder/b.txt": "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
		"test_data/test_subfolder3/Zest.py":              "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
		"test_data/test_subfolder3/best.py":              "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
		"test_data/test_subfolder3/test.py":              "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
		"test_data/test_subfolder4/TEST_BUILD":           "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
		"test_data/test_subfolder4/test.py":              "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
		"walk.go":                                        "8fd7339e6e8f7d203e61b7774fdef7692eb9c723",
	}
	assert.EqualValues(t, expected, b1)
}

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
