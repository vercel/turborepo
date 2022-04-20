package globby

import (
	"os"
	"path/filepath"
	"strings"

	"github.com/bmatcuk/doublestar/v4"
	"github.com/spf13/afero"
)

var afs *afero.Afero

func init() {
	setFileSystem(afero.NewOsFs())
}

func getFileSystem() afero.Fs {
	return afs
}

func setFileSystem(fs afero.Fs) {
	afs = &afero.Afero{Fs: fs}
}

func GlobFiles(basePath string, includePatterns []string, excludePatterns []string) []string {
	var include []string
	var exclude []string
	var result []string

	for _, p := range includePatterns {
		include = append(include, filepath.Join(basePath, p))
	}

	for _, p := range excludePatterns {
		exclude = append(exclude, filepath.Join(basePath, p))
	}

	includePattern := "{" + strings.Join(include, ",") + "}"
	excludePattern := "{" + strings.Join(exclude, ",") + "}"

	_ = afs.Walk(basePath, func(path string, info os.FileInfo, err error) error {
		var isDir = info.IsDir()
		if val, _ := doublestar.PathMatch(excludePattern, path); val {
			if isDir {
				return filepath.SkipDir
			}
			return nil
		}

		if isDir {
			return nil
		}

		if val, _ := doublestar.PathMatch(includePattern, path); val || len(includePatterns) == 0 {
			result = append(result, path)
		}

		return nil
	})

	return result
}
