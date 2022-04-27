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

// getRelativePath ensures that the the requested file path is a child of `from`.
func getRelativePath(from string, to string) (path string, err error) {
	relativePath, err := filepath.Rel(from, to)

	if err != nil {
		return "", err
	}

	if strings.HasPrefix(relativePath, "..") {
		return "", errors.New("the path you are attempting to specify is outside of the root")
	}

	return relativePath, nil
}

// globFilesFs searches the specified file system to ensure to enumerate all files to include.
func globFilesFs(fs afero.IOFS, basePath string, includePatterns []string, excludePatterns []string) []string {
	var processedIncludes []string
	var processedExcludes []string
	result := make(util.Set)

	for _, includePattern := range includePatterns {
		includePath := filepath.Join(basePath, includePattern)
		relativePath, err := getRelativePath(basePath, includePath)

		if err == nil {
			// Includes only operate on files.
			processedIncludes = append(processedIncludes, relativePath)
		}
	}

	for _, excludePattern := range excludePatterns {
		excludePath := filepath.Join(basePath, excludePattern)
		relativePath, err := getRelativePath(basePath, excludePath)

		if err == nil {
			// Excludes operate on entire folders.
			processedExcludes = append(processedExcludes, filepath.Join(relativePath, "**"))
		}
	}

	// We start from a naive includePattern
	includePattern := ""
	includeCount := len(processedIncludes)

	// Do not use alternation if unnecessary.
	if includeCount == 1 {
		includePattern = filepath.Join(basePath, processedIncludes[0])
	} else if includeCount > 1 {
		// We start from a basePath prefix which allows doublestar to optimize the access.
		includePattern = filepath.Join(basePath, "{"+strings.Join(processedIncludes, ",")+"}")
	}

	// We only create an exclude pattern if we have excludes.
	var excludePattern string
	excludeCount := len(processedExcludes)

	// Do not use alternation if unnecessary.
	if excludeCount == 1 {
		excludePattern = filepath.Join(basePath, processedExcludes[0])
	} else if excludeCount > 1 {
		// We start from a basePath prefix which allows doublestar to optimize the access.
		excludePattern = filepath.Join(basePath, "{"+strings.Join(processedExcludes, ",")+"}")
	}

	err := doublestar.GlobWalk(fs, filepath.ToSlash(includePattern), func(path string, dirEntry iofs.DirEntry) error {
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

		isExcluded, err := doublestar.Match(filepath.ToSlash(excludePattern), filepath.ToSlash(path))

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
