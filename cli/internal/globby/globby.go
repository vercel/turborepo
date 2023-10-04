package globby

import (
	"fmt"
	"path/filepath"
	"sort"
	"strings"

	iofs "io/fs"

	"github.com/vercel/turbo/cli/internal/fs"

	"github.com/vercel/turbo/cli/internal/doublestar"
	"github.com/vercel/turbo/cli/internal/util"
)

// GlobAll returns an array of files and folders that match the specified set of glob patterns.
// The returned files and folders are absolute paths, assuming that basePath is an absolute path.
func GlobAll(basePath string, includePatterns []string, excludePatterns []string) ([]string, error) {
	fsys := fs.CreateDirFSAtRoot(basePath)
	fsysRoot := fs.GetDirFSRootPath(fsys)
	output, err := globAllFs(fsys, fsysRoot, basePath, includePatterns, excludePatterns)

	// Because this is coming out of a map output is in no way ordered.
	// Sorting will put the files in a depth-first order.
	sort.Strings(output)
	return output, err
}

// GlobFiles returns an array of files that match the specified set of glob patterns.
// The return files are absolute paths, assuming that basePath is an absolute path.
func GlobFiles(basePath string, includePatterns []string, excludePatterns []string) ([]string, error) {
	fsys := fs.CreateDirFSAtRoot(basePath)
	fsysRoot := fs.GetDirFSRootPath(fsys)
	output, err := globFilesFs(fsys, fsysRoot, basePath, includePatterns, excludePatterns)

	// Because this is coming out of a map output is in no way ordered.
	// Sorting will put the files in a depth-first order.
	sort.Strings(output)
	return output, err
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

// globFilesFs searches the specified file system to enumerate all files to include.
func globFilesFs(fsys iofs.FS, fsysRoot string, basePath string, includePatterns []string, excludePatterns []string) ([]string, error) {
	return globWalkFs(fsys, fsysRoot, basePath, includePatterns, excludePatterns, false)
}

// globAllFs searches the specified file system to enumerate all files to include.
func globAllFs(fsys iofs.FS, fsysRoot string, basePath string, includePatterns []string, excludePatterns []string) ([]string, error) {
	return globWalkFs(fsys, fsysRoot, basePath, includePatterns, excludePatterns, true)
}

// globWalkFs searches the specified file system to enumerate all files and folders to include.
func globWalkFs(fsys iofs.FS, fsysRoot string, basePath string, includePatterns []string, excludePatterns []string, includeDirs bool) ([]string, error) {
	var processedIncludes []string
	var processedExcludes []string
	result := make(util.Set)

	for _, includePattern := range includePatterns {
		includePath := filepath.Join(basePath, includePattern)
		err := checkRelativePath(basePath, includePath)

		if err != nil {
			return nil, err
		}

		// fs.FS paths may not include leading separators. Calculate the
		// correct path for this relative to the filesystem root.
		// This will not error as it follows the call to checkRelativePath.
		iofsRelativePath, _ := fs.IofsRelativePath(fsysRoot, includePath)

		// Includes only operate on files.
		processedIncludes = append(processedIncludes, iofsRelativePath)
	}

	for _, excludePattern := range excludePatterns {
		excludePath := filepath.Join(basePath, excludePattern)
		err := checkRelativePath(basePath, excludePath)

		if err != nil {
			return nil, err
		}

		// fs.FS paths may not include leading separators. Calculate the
		// correct path for this relative to the filesystem root.
		// This will not error as it follows the call to checkRelativePath.
		iofsRelativePath, _ := fs.IofsRelativePath(fsysRoot, excludePath)

		// In case this is a file pattern and not a directory, add the exact pattern.
		// In the event that the user has already specified /**,
		if !strings.HasSuffix(iofsRelativePath, string(filepath.Separator)+"**") {
			processedExcludes = append(processedExcludes, iofsRelativePath)
		}
		// TODO: we need to either document or change this behavior
		// Excludes operate on entire folders, so we also exclude everything under this in case it represents a directory
		processedExcludes = append(processedExcludes, filepath.Join(iofsRelativePath, "**"))
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

	err := doublestar.GlobWalk(fsys, includePattern, func(path string, dirEntry iofs.DirEntry) error {
		if !includeDirs && dirEntry.IsDir() {
			return nil
		}

		// All files that are returned by doublestar.GlobWalk are relative to
		// the fsys root. Go, however, has decided that `fs.FS` filesystems do
		// not address the root of the file system using `/` and instead use
		// paths without leading separators.
		//
		// We need to track where the `fsys` root is so that when we hand paths back
		// we hand them back as the path addressable in the actual OS filesystem.
		//
		// As a consequence, when processing, we need to *restore* the original
		// root to the file path after returning. This works because when we create
		// the `os.dirFS` filesystem we do so at the root of the current volume.
		if excludeCount == 0 {
			// Reconstruct via string concatenation since the root is already pre-composed.
			result.Add(fsysRoot + path)
			return nil
		}

		isExcluded, err := doublestar.Match(excludePattern, filepath.ToSlash(path))
		if err != nil {
			return err
		}

		if !isExcluded {
			// Reconstruct via string concatenation since the root is already pre-composed.
			result.Add(fsysRoot + path)
		}

		return nil
	})

	// GlobWalk threw an error.
	if err != nil {
		return nil, err
	}

	// Never actually capture the root folder.
	// This is a risk because of how we rework the globs.
	result.Delete(strings.TrimSuffix(basePath, "/"))

	return result.UnsafeListOfStrings(), nil
}
