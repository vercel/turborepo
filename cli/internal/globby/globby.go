package globby

import (
	"github.com/vercel/turborepo/cli/internal/fs"

	"path/filepath"
	"strings"

	"github.com/bmatcuk/doublestar/v4"
)

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
	_ = fs.Walk(basePath, func(p string, isDir bool) error {
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
