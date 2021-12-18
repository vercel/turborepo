package globby

import (
	"os"
	"path/filepath"
	"strings"
	"turbo/internal/util"

	"github.com/bmatcuk/doublestar/v4"
	"github.com/karrick/godirwalk"
)

// IgnoreFunc checks if a path ought to be ignored
type IgnoreFunc func(path string) bool

// IgnoreNone ignores nothing
var IgnoreNone IgnoreFunc = func(path string) bool { return false }

// IgnoreStrings ignores all paths which contain one of the ignores substrings
func IgnoreStrings(ignores []string) IgnoreFunc {
	return func(path string) bool {
		for _, ptn := range ignores {
			if ptn == "" {
				continue
			}
			if strings.Contains(path, ptn) {
				return true
			}
		}
		return false
	}
}
func Globby(baseDir string, patterns []string) ([]string, error) {
	var filesToBeCached = make(util.Set)
	for _, output := range patterns {
		results, err := doublestar.Glob(os.DirFS(baseDir), strings.TrimPrefix(output, "!"))
		if err != nil {
			return nil, err
		}
		for _, result := range results {
			if strings.HasPrefix(output, "!") {
				filesToBeCached.Delete(result)
			} else {
				filesToBeCached.Add(result)
			}
		}
	}
	return filesToBeCached.UnsafeListOfStrings(), nil
}

// Glob finds all files that match the pattern and not the ignore func
func Glob(base, pattern string, ignore IgnoreFunc) ([]string, error) {
	var res []string
	err := godirwalk.Walk(base, &godirwalk.Options{
		Callback: func(osPathname string, directoryEntry *godirwalk.Dirent) error {
			if ignore != nil && ignore(osPathname) {
				if directoryEntry.IsDir() {
					return filepath.SkipDir
				}
				return nil
			}

			path := strings.TrimPrefix(osPathname, base+"/")
			m, err := Match(pattern, path)
			if err != nil {
				return err
			}
			if m {
				res = append(res, osPathname)
			}
			return nil
		},
		FollowSymbolicLinks: true,
		Unsorted:            true,
		ErrorCallback: func(path string, err error) godirwalk.ErrorAction {
			return godirwalk.SkipNode
		},
	})
	if err != nil {
		return nil, err
	}

	return res, nil
}

// Match matches the same patterns as filepath.Match except it can also match
// an arbitrary number of path segments using **
func Match(pattern, path string) (matches bool, err error) {
	if path == pattern {
		return true, nil
	}

	var (
		patterns = strings.Split(filepath.ToSlash(pattern), "/")
		paths    = strings.Split(filepath.ToSlash(path), "/")
	)
	return match(patterns, paths)
}

func match(patterns, paths []string) (matches bool, err error) {
	var pathIndex int
	for patternIndex := 0; patternIndex < len(patterns); patternIndex++ {
		pattern := patterns[patternIndex]
		if patternIndex >= len(paths) {
			// pattern is longer than path - path can't match
			// TODO: what if the last pattern segment is **
			return false, nil
		}

		path := paths[pathIndex]
		if pattern == path {
			// path and pattern segment match exactly - consume the path segment
			pathIndex++
			continue
		}

		if pattern == "**" {
			if patternIndex == len(patterns)-1 {
				// this is the last pattern segment, hence we consume the remainder of the path.
				return true, nil
			}

			// this segment consumes all path segments until the next pattern segment
			nextPattern := patterns[patternIndex+1]
			if nextPattern == "**" {
				// next pattern is a doublestar, too. Hence we just consume this path segment
				// and let the next doublestar do the work.
				continue
			}

			// we consume one path segment after the other and check if the remainder of the pattern
			// matches the remainder of the path
			for pi := pathIndex; pi < len(paths); pi++ {
				m, err := match(patterns[patternIndex+1:], paths[pi:])
				if err != nil {
					return false, err
				}
				if m {
					return true, nil
				}
			}
			// none of the remainder matched
			return false, nil
		}

		match, err := filepath.Match(pattern, path)
		if err != nil {
			return false, err
		}
		if match {
			pathIndex++
			continue
		}

		// did not find a match - we're done here
		return false, nil
	}

	// we made it through the whole pattern, which means it matches alright
	return true, nil
}
