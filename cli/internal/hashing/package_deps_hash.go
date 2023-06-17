package hashing

import (
	"bufio"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"

	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/encoding/gitoutput"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/util"
)

// PackageDepsOptions are parameters for getting git hashes for a filesystem
type PackageDepsOptions struct {
	// PackagePath is the folder path to derive the package dependencies from. This is typically the folder
	// containing package.json. If omitted, the default value is the current working directory.
	PackagePath turbopath.AnchoredSystemPath

	InputPatterns []string
}

// GetPackageFileHashes Builds an object containing git hashes for the files under the specified `packagePath` folder.
func GetPackageFileHashes(rootPath turbopath.AbsoluteSystemPath, packagePath turbopath.AnchoredSystemPath, inputs []string) (map[turbopath.AnchoredUnixPath]string, error) {
	if len(inputs) == 0 {
		result, err := getPackageFileHashesFromGitIndex(rootPath, packagePath)
		if err != nil {
			return getPackageFileHashesFromProcessingGitIgnore(rootPath, packagePath, nil)
		}
		return result, nil
	}

	result, err := getPackageFileHashesFromInputs(rootPath, packagePath, inputs)
	if err != nil {
		return getPackageFileHashesFromProcessingGitIgnore(rootPath, packagePath, inputs)
	}
	return result, nil
}

// GetHashesForFiles hashes the list of given files, then returns a map of normalized path to hash.
// This map is suitable for cross-platform caching.
func GetHashesForFiles(rootPath turbopath.AbsoluteSystemPath, files []turbopath.AnchoredSystemPath) (map[turbopath.AnchoredUnixPath]string, error) {
	// Try to use `git` first.
	gitHashedFiles, err := gitHashObject(rootPath, files)
	if err == nil {
		return gitHashedFiles, nil
	}

	// Fall back to manual hashing.
	return manuallyHashFiles(rootPath, files, false)
}

// GetHashesForExistingFiles hashes the list of given files,
// does not error if a file does not exist, then
// returns a map of normalized path to hash.
// This map is suitable for cross-platform caching.
func GetHashesForExistingFiles(rootPath turbopath.AbsoluteSystemPath, files []turbopath.AnchoredSystemPath) (map[turbopath.AnchoredUnixPath]string, error) {
	return manuallyHashFiles(rootPath, files, true)
}

// gitHashObject returns a map of paths to their SHA hashes calculated by passing the paths to `git hash-object`.
// `git hash-object` expects paths to use Unix separators, even on Windows.
//
// Note: paths of files to hash passed to `git hash-object` are processed as relative to the given anchor.
// For that reason we convert all input paths and make them relative to the anchor prior to passing them
// to `git hash-object`.
func gitHashObject(anchor turbopath.AbsoluteSystemPath, filesToHash []turbopath.AnchoredSystemPath) (map[turbopath.AnchoredUnixPath]string, error) {
	fileCount := len(filesToHash)
	output := make(map[turbopath.AnchoredUnixPath]string, fileCount)

	if fileCount > 0 {
		cmd := exec.Command(
			"git",           // Using `git` from $PATH,
			"hash-object",   // hash a file,
			"--stdin-paths", // using a list of newline-separated paths from stdin.
		)
		cmd.Dir = anchor.ToString() // Start at this directory.

		// The functionality for gitHashObject is different enough that it isn't reasonable to
		// generalize the behavior for `runGitCmd`. In fact, it doesn't even use the `gitoutput`
		// encoding library, instead relying on its own separate `bufio.Scanner`.

		// We're going to send the list of files in via `stdin`, so we grab that pipe.
		// This prevents a huge number of encoding issues and shell compatibility issues
		// before they even start.
		stdinPipe, stdinPipeError := cmd.StdinPipe()
		if stdinPipeError != nil {
			return nil, stdinPipeError
		}

		// Kick the processing off in a goroutine so while that is doing its thing we can go ahead
		// and wire up the consumer of `stdout`.
		go func() {
			defer util.CloseAndIgnoreError(stdinPipe)

			// `git hash-object` understands all relative paths to be relative to the repository.
			// This function's result needs to be relative to `rootPath`.
			// We convert all files to absolute paths and assume that they will be inside of the repository.
			for _, file := range filesToHash {
				converted := file.RestoreAnchor(anchor)

				// `git hash-object` expects paths to use Unix separators, even on Windows.
				// `git hash-object` expects paths to be one per line so we must escape newlines.
				// In order to understand the escapes, the path must be quoted.
				// In order to quote the path, the quotes in the path must be escaped.
				// Other than that, we just write everything with full Unicode.
				stringPath := converted.ToString()
				toSlashed := filepath.ToSlash(stringPath)
				escapedNewLines := strings.ReplaceAll(toSlashed, "\n", "\\n")
				escapedQuotes := strings.ReplaceAll(escapedNewLines, "\"", "\\\"")
				prepared := fmt.Sprintf("\"%s\"\n", escapedQuotes)
				_, err := io.WriteString(stdinPipe, prepared)
				if err != nil {
					return
				}
			}
		}()

		// This gives us an io.ReadCloser so that we never have to read the entire input in
		// at a single time. It is doing stream processing instead of string processing.
		stdoutPipe, stdoutPipeError := cmd.StdoutPipe()
		if stdoutPipeError != nil {
			return nil, fmt.Errorf("failed to read `git hash-object`: %w", stdoutPipeError)
		}

		startError := cmd.Start()
		if startError != nil {
			return nil, fmt.Errorf("failed to read `git hash-object`: %w", startError)
		}

		// The output of `git hash-object` is a 40-character SHA per input, then a newline.
		// We need to track the SHA that corresponds to the input file path.
		index := 0
		hashes := make([]string, len(filesToHash))
		scanner := bufio.NewScanner(stdoutPipe)

		// Read the output line-by-line (which is our separator) until exhausted.
		for scanner.Scan() {
			bytes := scanner.Bytes()

			scanError := scanner.Err()
			if scanError != nil {
				return nil, fmt.Errorf("failed to read `git hash-object`: %w", scanError)
			}

			hashError := gitoutput.CheckObjectName(bytes)
			if hashError != nil {
				return nil, fmt.Errorf("failed to read `git hash-object`: %s", "invalid hash received")
			}

			// Worked, save it off.
			hashes[index] = string(bytes)
			index++
		}

		// Waits until stdout is closed before proceeding.
		waitErr := cmd.Wait()
		if waitErr != nil {
			return nil, fmt.Errorf("failed to read `git hash-object`: %w", waitErr)
		}

		// Make sure we end up with a matching number of files and hashes.
		hashCount := len(hashes)
		if fileCount != hashCount {
			return nil, fmt.Errorf("failed to read `git hash-object`: %d files %d hashes", fileCount, hashCount)
		}

		// The API of this method specifies that we return a `map[turbopath.AnchoredUnixPath]string`.
		for i, hash := range hashes {
			filePath := filesToHash[i]
			output[filePath.ToUnixPath()] = hash
		}
	}

	return output, nil
}

func manuallyHashFiles(rootPath turbopath.AbsoluteSystemPath, files []turbopath.AnchoredSystemPath, allowMissing bool) (map[turbopath.AnchoredUnixPath]string, error) {
	hashObject := make(map[turbopath.AnchoredUnixPath]string, len(files))
	for _, file := range files {
		hash, err := fs.GitLikeHashFile(file.RestoreAnchor(rootPath))
		if allowMissing && errors.Is(err, os.ErrNotExist) {
			continue
		}
		if err != nil {
			return nil, fmt.Errorf("could not hash file %v. \n%w", file.ToString(), err)
		}

		hashObject[file.ToUnixPath()] = hash
	}
	return hashObject, nil
}

// getTraversePath gets the distance of the current working directory to the repository root.
// This is used to convert repo-relative paths to cwd-relative paths.
//
// `git rev-parse --show-cdup` always returns Unix paths, even on Windows.
func getTraversePath(rootPath turbopath.AbsoluteSystemPath) (turbopath.RelativeUnixPath, error) {
	cmd := exec.Command("git", "rev-parse", "--show-cdup")
	cmd.Dir = rootPath.ToString()

	traversePath, err := cmd.Output()
	if err != nil {
		return "", err
	}

	trimmedTraversePath := strings.TrimSuffix(string(traversePath), "\n")

	return turbopath.RelativeUnixPathFromUpstream(trimmedTraversePath), nil
}

// Don't shell out if we already know where you are in the repository.
// `memoize` is a good candidate for generics.
func memoizeGetTraversePath() func(turbopath.AbsoluteSystemPath) (turbopath.RelativeUnixPath, error) {
	cacheMutex := &sync.RWMutex{}
	cachedResult := map[turbopath.AbsoluteSystemPath]turbopath.RelativeUnixPath{}
	cachedError := map[turbopath.AbsoluteSystemPath]error{}

	return func(rootPath turbopath.AbsoluteSystemPath) (turbopath.RelativeUnixPath, error) {
		cacheMutex.RLock()
		result, resultExists := cachedResult[rootPath]
		err, errExists := cachedError[rootPath]
		cacheMutex.RUnlock()

		if resultExists && errExists {
			return result, err
		}

		invokedResult, invokedErr := getTraversePath(rootPath)
		cacheMutex.Lock()
		cachedResult[rootPath] = invokedResult
		cachedError[rootPath] = invokedErr
		cacheMutex.Unlock()

		return invokedResult, invokedErr
	}
}

var memoizedGetTraversePath = memoizeGetTraversePath()
