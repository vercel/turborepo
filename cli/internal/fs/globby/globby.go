package globby

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
)

type Option struct {
	BaseDir        string
	CheckDot       bool
	RelativeReturn bool
	Excludes       []string
}

/*
 * Glob all patterns
 */
func Match(patterns []string, opt Option) []string {
	var allFiles []string
	patterns, opt, err := completeOpt(patterns, opt)
	if err != nil {
		fmt.Printf("Magth err: [%v]\n", err)
		return allFiles
	}
	for _, pattern := range patterns {
		files := find(pattern, opt)
		if files == nil || len(*files) == 0 {
			continue
		}
		allFiles = append(allFiles, *files...)
	}
	return allFiles
}

func find(pattern string, opt Option) *[]string {
	// match ./some/path/**/*
	if regexTest("\\*\\*", pattern) ||
		!regexTest("\\*", pattern) { // Dirname
		return findRecr(pattern, opt)
	}
	// match ./some/path/*
	if regexTest("\\*", pattern) {
		return findDir(pattern, opt)
	}
	return nil
}

// find under centain directory
func findDir(pattern string, opt Option) *[]string {
	var list []string
	files, err := filepath.Glob(pattern)
	if err != nil {
		fmt.Printf("err: [%v]\n", err)
		return &list
	}
	for _, fullpath := range files {
		path, err := filepath.Rel(opt.BaseDir, fullpath)
		if err != nil {
			continue
		}
		if checkExclude(opt, path) {
			continue
		}
		if opt.RelativeReturn {
			list = append(list, path)
		} else {
			list = append(list, fullpath)
		}
	}
	return &list
}

// find recursively
func findRecr(pattern string, opt Option) *[]string {
	dir := strReplace(pattern, "\\*\\*.+", "")
	afterMacth := ""
	matchAfterFlag := false
	if regexTest("\\*", pattern) {
		afterMacth = strReplace(pattern, ".+\\*", "")
		matchAfterFlag = len(afterMacth) > 0
	}

	var list []string
	err := filepath.Walk(dir, func(fullpath string, f os.FileInfo, err error) error {
		if !opt.CheckDot && regexTest("^\\.", f.Name()) {
			if f.IsDir() {
				return filepath.SkipDir
			}
			return nil
		}
		if f.IsDir() {
			return nil
		}
		path, _ := filepath.Rel(opt.BaseDir, fullpath)
		if checkExclude(opt, path) {
			return nil
		}
		if !opt.RelativeReturn {
			path = fullpath
		}
		if !matchAfterFlag {
			list = append(list, path)
			return nil
		}
		if regexTest(afterMacth+"$", path) {
			list = append(list, path)
		}
		return nil
	})
	if err != nil {
		fmt.Printf("err: [%v]\n", err)
	}
	return &list
}

// check and complete the options
func completeOpt(srcPatterns []string, opt Option) ([]string, Option, error) {
	if len(opt.BaseDir) == 0 {
		curDir, err := os.Getwd()
		if err != nil {
			panic(err)
		}
		opt.BaseDir = curDir
	}

	var patterns []string
	for _, pattern := range srcPatterns {
		// TODO: check no "tmp/*", use "tmp" or "tmp/*.ext" instead

		if regexTest("^\\!", pattern) {
			opt.Excludes = append(opt.Excludes, strReplace(pattern, "^\\!", ""))
			continue
		}
		if regexTest("^\\.", pattern) || // like ./dist
			!regexTest("^\\/", pattern) { // like dist
			patterns = append(patterns, filepath.Join(opt.BaseDir, pattern))
			continue
		}
		patterns = append(patterns, pattern)
	}
	return patterns, opt, nil
}

// check if path should be excluded
func checkExclude(opt Option, path string) bool {
	// if exludes dirs
	for _, exclude := range opt.Excludes {
		rule := exclude
		if regexTest("\\*\\*", exclude) {
			rule = strReplace(exclude, "\\*\\*/\\*+?", ".+")
		} else if regexTest("\\*", exclude) {
			rule = strReplace(exclude, "\\*", "[^/]+")
		}
		if regexTest("^"+rule, path) {
			return true // ignore
		}
	}
	return false
}

// Check if regex match the "src" string
func regexTest(re string, src string) bool {
	matched, err := regexp.MatchString(re, src)
	if err != nil {
		return false
	}
	if matched {
		return true
	}
	return false
}

// "dest" replace "text" pattern with "repl"
func strReplace(dest, text, repl string) string {
	re := regexp.MustCompile(text)
	return re.ReplaceAllString(dest, repl)
}
