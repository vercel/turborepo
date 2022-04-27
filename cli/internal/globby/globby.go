package globby

import (
	"errors"
	"path/filepath"
	"strings"

	iofs "io/fs"

	"github.com/bmatcuk/doublestar/v4"
	"github.com/spf13/afero"
	"github.com/vercel/turborepo/cli/internal/util"
)

var _aferoOsFs = afero.NewOsFs()
var _aferoIOFS = afero.NewIOFS(_aferoOsFs)

func GlobFiles(basePath string, includePatterns []string, excludePatterns []string) []string {
	return globFilesFs(_aferoIOFS, basePath, includePatterns, excludePatterns)
}

// isRelativePath ensures that the the requested file path is a child of `from`.
func isRelativePath(from string, to string) (isRelative bool, err error) {
	relativePath, err := filepath.Rel(from, to)

	if err != nil {
		return false, err
	}

	if strings.HasPrefix(relativePath, "..") {
		return false, errors.New("the path you are attempting to specify is outside of the root")
	}

	return true, nil
}

// globFilesFs searches the specified file system to ensure to enumerate all files to include.
func globFilesFs(fs afero.IOFS, basePath string, includePatterns []string, excludePatterns []string) []string {
	var processedIncludes []string
	var processedExcludes []string
	result := make(util.Set)

	for _, includePattern := range includePatterns {
		includePath := filepath.Join(basePath, includePattern)
		isRelative, _ := isRelativePath(basePath, includePath)

		if isRelative {
			// Includes only operate on files.
			processedIncludes = append(processedIncludes, includePath)
		}
	}

	for _, excludePattern := range excludePatterns {
		excludePath := filepath.Join(basePath, excludePattern)
		isRelative, _ := isRelativePath(basePath, excludePath)

		if isRelative {
			// Excludes operate on entire folders.
			processedExcludes = append(processedExcludes, filepath.Join(excludePath, "**"))
		}
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

		if err == nil && !isExcluded {
			result.Add(path)
		}

		return nil
	})

	if err != nil {
		return nil
	}

	return result.UnsafeListOfStrings()
}
