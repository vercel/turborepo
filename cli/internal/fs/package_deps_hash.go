package fs

import (
	"bytes"
	"fmt"
	"io"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"

	"github.com/pkg/errors"
	"github.com/vercel/turborepo/cli/internal/encoding/gitoutput"
)

// PackageDepsOptions are parameters for getting git hashes for a filesystem
type PackageDepsOptions struct {
	// PackagePath is the folder path to derive the package dependencies from. This is typically the folder
	// containing package.json. If omitted, the default value is the current working directory.
	PackagePath string

	InputPatterns []string
}

// GetPackageDeps Builds an object containing git hashes for the files under the specified `packagePath` folder.
func GetPackageDeps(repoRoot AbsolutePath, p *PackageDepsOptions) (map[RepoRelativeUnixPath]string, error) {
	// Add all the checked in hashes.
	var result map[RepoRelativeUnixPath]string
	if len(p.InputPatterns) == 0 {
		gitLsTreeOutput, err := gitLsTree(p.PackagePath)
		if err != nil {
			return nil, fmt.Errorf("could not get git hashes for files in package %s: %w", p.PackagePath, err)
		}
		result = gitLsTreeOutput
	} else {
		gitLsFilesOutput, err := gitLsFiles(repoRoot.Join(p.PackagePath), p.InputPatterns)
		if err != nil {
			return nil, fmt.Errorf("could not get git hashes for file patterns %v in package %s: %w", p.InputPatterns, p.PackagePath, err)
		}
		result = gitLsFilesOutput
	}

	// Update the checked in hashes with the current repo status
	gitStatusOutput, err := gitStatus(repoRoot.Join(p.PackagePath), p.InputPatterns)
	if err != nil {
		return nil, fmt.Errorf("Could not get git hashes from git status")
	}

	var filesToHash []string
	for filePath, status := range gitStatusOutput {
		if status.x == "D" || status.y == "D" {
			delete(result, filePath)
		} else {
			filesToHash = append(filesToHash, filepath.Join(p.PackagePath, filePath.ToString()))
		}
	}
	hashes, err := GetHashableDeps(filesToHash, p.PackagePath)
	if err != nil {
		return nil, err
	}

	for platformIndependentPath, hash := range hashes {
		result[UnsafeToRepoRelativeUnixPath(platformIndependentPath)] = hash
	}
	return result, nil
}

// GetHashableDeps hashes the list of given files, then returns a map of normalized path to hash
// this map is suitable for cross-platform caching.
func GetHashableDeps(absolutePaths []string, relativeTo string) (map[string]string, error) {
	fileHashes, err := gitHashForFiles(absolutePaths)
	if err != nil {
		return nil, errors.Wrapf(err, "failed to hash files %v", strings.Join(absolutePaths, ", "))
	}
	result := make(map[string]string)
	for filename, hash := range fileHashes {
		// Normalize path as POSIX-style and relative to "relativeTo"
		relativePath, err := filepath.Rel(relativeTo, filename)
		if err != nil {
			return nil, errors.Wrapf(err, "failed to get relative path from %v to %v", relativeTo, relativePath)
		}
		key := filepath.ToSlash(relativePath)
		result[key] = hash
	}
	return result, nil
}

// threadsafeBufferWriter is a wrapper around a byte buffer and a lock, allowing
// multiple goroutines to write to the same buffer. No attempt is made to
// lock around reading, which should only be done once no more writing will occur.
type threadsafeBufferWriter struct {
	mu     sync.Mutex
	buffer bytes.Buffer
}

func (tsbw *threadsafeBufferWriter) Write(p []byte) (int, error) {
	tsbw.mu.Lock()
	defer tsbw.mu.Unlock()
	return tsbw.buffer.Write(p)
}

var _ io.Writer = (*threadsafeBufferWriter)(nil)

// gitHashForFiles a list of files returns a map of with their git hash values. It uses
// git hash-object under the hood.
// Note that filesToHash must have full paths.
func gitHashForFiles(filesToHash []string) (map[string]string, error) {
	changes := make(map[string]string)
	if len(filesToHash) > 0 {
		input := []string{"hash-object"}
		input = append(input, filesToHash...)
		cmd := exec.Command("git", input...)
		// exec.Command writes to stdout and stderr from different goroutines,
		// but only if they aren't the same io.Writer. Since we're passing different
		// io.Writer instances that can optionally write to the same buffer, we need
		// to do the locking ourselves.
		var allout threadsafeBufferWriter
		var stdout bytes.Buffer
		// write stdout to both the stdout buffer, and the combined output buffer
		// we don't expect to need stderr, but in the event that something goes wrong,
		// we'd like to report the entirety of the output in the order it was output.
		mw := io.MultiWriter(&allout, &stdout)
		cmd.Stdout = mw
		cmd.Stderr = &allout
		err := cmd.Run()
		if err != nil {
			output := string(allout.buffer.Bytes())
			return nil, fmt.Errorf("git hash-object exited with status: %w. Output:\n%v", err, output)
		}
		offByOne := strings.Split(string(stdout.Bytes()), "\n") // there is an extra ""
		hashes := offByOne[:len(offByOne)-1]
		if len(hashes) != len(filesToHash) {
			output := string(allout.buffer.Bytes())
			return nil, fmt.Errorf("passed %v file paths to Git to hash, but received %v hashes. Full output:\n%v", len(filesToHash), len(hashes), output)
		}
		for i, hash := range hashes {
			filepath := filesToHash[i]
			changes[filepath] = hash
		}
	}

	return changes, nil
}

func runGitCommand(cmd *exec.Cmd, name string, handler func(io.Reader) *gitoutput.Reader) ([][]string, error) {
	out, pipeError := cmd.StdoutPipe()
	if pipeError != nil {
		return nil, fmt.Errorf("failed to read `git %s`: %w", name, pipeError)
	}

	startError := cmd.Start()
	if startError != nil {
		return nil, fmt.Errorf("failed to read `git %s`: %w", name, startError)
	}

	reader := handler(out)
	entries, readErr := reader.ReadAll()
	if readErr != nil {
		return nil, fmt.Errorf("failed to read `git %s`: %w", name, readErr)
	}

	waitErr := cmd.Wait()
	if waitErr != nil {
		return nil, fmt.Errorf("failed to read `git %s`: %w", name, waitErr)
	}

	return entries, nil
}

func gitLsTree(path string) (map[RepoRelativeUnixPath]string, error) {
	cmd := exec.Command("git", "ls-tree", "HEAD", "-z", "-r")
	cmd.Dir = path

	entries, err := runGitCommand(cmd, "ls-tree", gitoutput.NewLSTreeReader)
	if err != nil {
		return nil, err
	}

	changes := make(map[RepoRelativeUnixPath]string, len(entries))

	for _, entry := range entries {
		changes[UnsafeToRepoRelativeUnixPath(entry[3])] = entry[2]
	}

	return changes, nil
}

func gitLsFiles(path AbsolutePath, patterns []string) (map[RepoRelativeUnixPath]string, error) {
	cmd := exec.Command("git", "ls-files", "-s", "-z", "--")
	cmd.Args = append(cmd.Args, patterns...)
	cmd.Dir = path.ToString()

	entries, err := runGitCommand(cmd, "ls-files", gitoutput.NewLSFilesReader)
	if err != nil {
		return nil, err
	}

	changes := make(map[RepoRelativeUnixPath]string, len(entries))

	for _, entry := range entries {
		changes[UnsafeToRepoRelativeUnixPath(entry[3])] = entry[1]
	}

	return changes, nil
}

type status struct {
	x string
	y string
}

func gitStatus(path AbsolutePath, patterns []string) (map[RepoRelativeUnixPath]status, error) {
	cmd := exec.Command("git", "status", "-u", "-z", "--")
	if len(patterns) == 0 {
		cmd.Args = append(cmd.Args, ".")
	} else {
		cmd.Args = append(cmd.Args, patterns...)
	}
	cmd.Dir = path.ToString()

	entries, err := runGitCommand(cmd, "status", gitoutput.NewStatusReader)
	if err != nil {
		return nil, err
	}

	changes := make(map[RepoRelativeUnixPath]status, len(entries))

	for _, entry := range entries {
		changes[UnsafeToRepoRelativeUnixPath(entry[2])] = status{x: entry[0], y: entry[1]}
	}

	return changes, nil
}
