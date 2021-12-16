package globby

import (
	"turbo/internal/fs"

	"path/filepath"
	"strings"

	"github.com/bmatcuk/doublestar/v4"
	"github.com/karrick/godirwalk"
)

func GlobFiles(ws_path string, include_pattens *[]string, exclude_pattens *[]string) []string {
	var include []string
	var exclude []string
	var result []string

	for _, p := range *include_pattens {
		include = append(include, filepath.Join(ws_path, p))
	}

	for _, p := range *exclude_pattens {
		exclude = append(exclude, filepath.Join(ws_path, p))
	}

	var include_pattern = "{" + strings.Join(include, ",") + "}"
	var exclude_pattern = "{" + strings.Join(exclude, ",") + "}"
	var _ = fs.Walk(ws_path, func(p string, isDir bool) error {
		if val, _ := doublestar.PathMatch(exclude_pattern, p); val {
			return godirwalk.SkipThis
		}

		if isDir {
			return nil
		}

		if val, _ := doublestar.PathMatch(include_pattern, p); val || len(*include_pattens) == 0 {
			result = append(result, p)
		}

		return nil
	})

	return result
}
