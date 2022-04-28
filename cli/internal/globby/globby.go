package globby

import (
	"fmt"
	"path/filepath"
	"strings"

	iofs "io/fs"

	"github.com/bmatcuk/doublestar/v4"
	"github.com/spf13/afero"
	"github.com/vercel/turborepo/cli/internal/util"
)

var _aferoOsFs = afero.NewOsFs()
var _aferoIOFS = afero.NewIOFS(_aferoOsFs)

// GlobFiles returns an array of files that match the specified set of glob patterns.
func GlobFiles(basePath string, includePatterns []string, excludePatterns []string) ([]string, error) {
	return globFilesFs(_aferoIOFS, basePath, includePatterns, excludePatterns)
}

// checkRelativePath ensures that the the requested file path is a child of `from`.
func checkRelativePath(from string, to string) error {
	relativePath, err := filepath.Rel(from, to)

	if err != nil {
		return err
	}

	if strings.HasPrefix(relativePath, "..") {
		return fmt.Errorf("the path you are attempting to specify (%s) is outside of the root", to)
	}

	return nil
}

// globFilesFs searches the specified file system to ensure to enumerate all files to include.
func globFilesFs(fs afero.IOFS, basePath string, includePatterns []string, excludePatterns []string) ([]string, error) {
	var processedIncludes []string
	var processedExcludes []string
	result := make(util.Set)

	for _, includePattern := range includePatterns {
		includePath := filepath.Join(basePath, includePattern)
		err := checkRelativePath(basePath, includePath)

		if err != nil {
			return nil, err
		}

		// Includes only operate on files.
		processedIncludes = append(processedIncludes, includePath)
	}

	for _, excludePattern := range excludePatterns {
		excludePath := filepath.Join(basePath, excludePattern)
		err := checkRelativePath(basePath, excludePath)

		if err != nil {
			return nil, err
		}

		// Excludes operate on entire folders.
		processedExcludes = append(processedExcludes, filepath.Join(excludePath, "**"))
	}

	// We start from a naive includePattern
	includePattern := ""
	includeCount := len(processedIncludes)

	// Do not use alternation if unnecessary.
	if includeCount == 1 {
		includePattern = processedIncludes[0]
	} else if includeCount > 1 {
		// We use alternation from the very root of the path. This avoids fs.Stat of the basePath.
		includePattern = "{" + strings.Join(processedIncludes, ",") + "}"
	}

	// We start with an empty string excludePattern which we only use if excludeCount > 0.
	excludePattern := ""
	excludeCount := len(processedExcludes)

	// Do not use alternation if unnecessary.
	if excludeCount == 1 {
		excludePattern = processedExcludes[0]
	} else if excludeCount > 1 {
		// We use alternation from the very root of the path. This avoids fs.Stat of the basePath.
		excludePattern = "{" + strings.Join(processedExcludes, ",") + "}"
	}

	// GlobWalk expects that everything uses Unix path conventions.
	includePattern = filepath.ToSlash(includePattern)
	excludePattern = filepath.ToSlash(excludePattern)

	err := doublestar.GlobWalk(fs, includePattern, func(path string, dirEntry iofs.DirEntry) error {
		// Unix root paths do not prepend the leading slash.
		if basePath == "/" && !strings.HasPrefix(path, "/") {
			path = filepath.Join(basePath, path)
		}

		if dirEntry.IsDir() {
			return nil
		}

		if excludeCount == 0 {
			result.Add(path)
			return nil
		}

		isExcluded, err := doublestar.Match(excludePattern, filepath.ToSlash(path))
		if err != nil {
			return err
		}

		if !isExcluded {
			result.Add(path)
		}

		return nil
	})

	// GlobWalk threw an error.
	if err != nil {
		return nil, err
	}

	return result.UnsafeListOfStrings(), nil
}
