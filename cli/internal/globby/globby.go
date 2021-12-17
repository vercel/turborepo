package globby

import (
	"turbo/internal/fs"

	"path/filepath"
	"strings"

	"github.com/bmatcuk/doublestar/v4"
)

func GlobFiles(ws_path string, includePatterns []string, excludePatterns []string) []string {
	var include []string
	var exclude []string
	var result []string

	for _, p := range includePatterns {
		include = append(include, filepath.Join(ws_path, p))
	}

	for _, p := range excludePatterns {
		exclude = append(exclude, filepath.Join(ws_path, p))
	}

	includePattern := "{" + strings.Join(include, ",") + "}"
	excludePattern := "{" + strings.Join(exclude, ",") + "}"
	_ = fs.Walk(ws_path, func(p string, isDir bool) error {
		if val, _ := doublestar.PathMatch(excludePattern, p); val {
			if isDir {
				return filepath.SkipDir
			}
			return nil
		}

		if isDir {
			return nil
		}

		if val, _ := doublestar.PathMatch(includePattern, p); val || len(includePatterns) == 0 {
			result = append(result, p)
		}

		return nil
	})

	return result
}
