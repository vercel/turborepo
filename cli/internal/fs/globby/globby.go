package globby

import (
	"fmt"
	"io/fs"
	"path/filepath"
	"strings"

	"github.com/bmatcuk/doublestar/v4"
)

// // GlobList accepts a list of doublestar directive globs and returns a list of files matching them
// func Globby(base string, globs []string) ([]string, error) {
// 	ignoreList := []string{}
// 	actualGlobs := []string{}
// 	for _, output := range globs {
// 		if strings.HasPrefix(output, "!") {
// 			ignoreList = append(ignoreList, strings.TrimPrefix(output, "!"))
// 		} else {
// 			actualGlobs = append(actualGlobs, output)
// 		}
// 	}
// 	files := []string{}
// 	for _, glob := range actualGlobs {
// 		matches, err := doublestar.Glob(os.DirFS(base), glob)
// 		if err != nil {
// 			return nil, err
// 		}
// 		for _, match := range matches {
// 			for _, ignore := range ignoreList {
// 				if isMatch, _ := doublestar.PathMatch(ignore, match); !isMatch {
// 					files = append(files, match)
// 				}
// 			}
// 		}
// 	}
// }

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
	var _ = filepath.Walk(ws_path, func(p string, info fs.FileInfo, err error) error {
		if err != nil {
			fmt.Printf("prevent panic by handling failure accessing a path %q: %v\n", p, err)
			return err
		}

		if val, _ := doublestar.PathMatch(exclude_pattern, p); val {
			if info.IsDir() {
				return filepath.SkipDir
			}
			return nil
		}

		if info.IsDir() {
			return nil
		}

		if val, _ := doublestar.PathMatch(include_pattern, p); val || len(*include_pattens) == 0 {
			result = append(result, p)
		}

		return nil
	})

	return result
}
