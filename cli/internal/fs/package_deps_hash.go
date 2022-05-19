package fs

import (
	"bufio"
	"fmt"
	"io"
	"os/exec"
	"path/filepath"
	"strings"

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
		gitLsTreeOutput, err := gitLsTree(repoRoot, p.PackagePath)
		if err != nil {
			return nil, fmt.Errorf("could not get git hashes for files in package %s: %w", p.PackagePath, err)
		}
		result = gitLsTreeOutput
	} else {
		gitLsFilesOutput, err := gitLsFiles(repoRoot, p.PackagePath, p.InputPatterns)
		if err != nil {
			return nil, fmt.Errorf("could not get git hashes for file patterns %v in package %s: %w", p.InputPatterns, p.PackagePath, err)
		}
		result = gitLsFilesOutput
	}

	// Update the checked in hashes with the current repo status
	gitStatusOutput, err := gitStatus(repoRoot, p.PackagePath, p.InputPatterns)
	if err != nil {
		return nil, fmt.Errorf("Could not get git hashes from git status")
	}

	var filesToHash []string
	for filePath, status := range gitStatusOutput {
		if status.x == "D" || status.y == "D" {
			delete(result, filePath)
		} else {
			filesToHash = append(filesToHash, filePath.ToString())
		}
	}

	hashes, err := gitHashObject(repoRoot, filesToHash)
	if err != nil {
		return nil, err
	}

	// Zip up file paths and hashes together
	for filePath, hash := range hashes {
		result[filePath] = hash
	}

	return result, nil
}

// GetHashableDeps hashes the list of given files, then returns a map of normalized path to hash
// this map is suitable for cross-platform caching.
func GetHashableDeps(repoRoot AbsolutePath, files []string) (map[RepoRelativeUnixPath]string, error) {
	result, hashError := gitHashObject(repoRoot, files)
	if hashError != nil {
		return nil, hashError
	}

	repoRootString := repoRoot.ToString()
	relativeResult := make(map[RepoRelativeUnixPath]string)
	for file, hash := range result {
		relativePath, err := filepath.Rel(repoRootString, file.ToString())
		if err != nil {
			return nil, err
		}
		relativeResult[UnsafeToRepoRelativeUnixPath(relativePath)] = hash
	}

	return relativeResult, nil
}

// gitHashObject takes a list of files returns a map of with their git hash values.
// It uses git hash-object under the hood.
// Note that filesToHash must have full paths.
func gitHashObject(repoRoot AbsolutePath, filesToHash []string) (map[RepoRelativeUnixPath]string, error) {
	fileCount := len(filesToHash)
	changes := make(map[RepoRelativeUnixPath]string, fileCount)

	if fileCount > 0 {
		cmd := exec.Command("git", "hash-object", "--stdin-paths")
		cmd.Dir = repoRoot.ToString()

		stdinPipe, stdinPipeError := cmd.StdinPipe()
		if stdinPipeError != nil {
			return nil, stdinPipeError
		}

		go func() {
			defer func() {
				stdinPipeCloseError := stdinPipe.Close()
				if stdinPipeCloseError != nil {
					return
				}
			}()

			for _, file := range filesToHash {
				// Expects paths to be one per line so we escape newlines
				_, err := io.WriteString(stdinPipe, strings.ReplaceAll(file, "\n", "\\n")+"\n")
				if err != nil {
					return
				}
			}
		}()

		stdoutPipe, stdoutPipeError := cmd.StdoutPipe()
		if stdoutPipeError != nil {
			return nil, fmt.Errorf("failed to read `git %s`: %w", "hash-object", stdoutPipeError)
		}

		startError := cmd.Start()
		if startError != nil {
			return nil, fmt.Errorf("failed to read `git %s`: %w", "hash-object", startError)
		}

		index := 0
		hashes := make([]string, len(filesToHash))
		scanner := bufio.NewScanner(stdoutPipe)

		for scanner.Scan() {
			hash := scanner.Text()
			scanError := scanner.Err()
			if scanError != nil {
				return nil, fmt.Errorf("failed to read `git %s`: %w", "hash-object", scanError)
			}

			// TODO: verify hash is SHA-like
			if len(hash) != 40 {
				return nil, fmt.Errorf("failed to read `git %s`: %s", "hash-object", "invalid hash received")
			}

			// Worked, save it off.
			hashes[index] = hash
			index++
		}

		waitErr := cmd.Wait()
		if waitErr != nil {
			return nil, fmt.Errorf("failed to read `git %s`: %w", "hash-object", waitErr)
		}

		hashCount := len(hashes)
		if fileCount != hashCount {
			return nil, fmt.Errorf("failed to read `git %s`: %d files, %d hashes", "hash-object", fileCount, hashCount)
		}

		for i, hash := range hashes {
			filepath := filesToHash[i]
			changes[UnsafeToRepoRelativeUnixPath(filepath)] = hash
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

func gitLsTree(repoRoot AbsolutePath, path string) (map[RepoRelativeUnixPath]string, error) {
	cmd := exec.Command("git", "ls-tree", "-r", "-z", "HEAD")
	cmd.Dir = filepath.Join(repoRoot.ToString(), path)

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

func gitLsFiles(repoRoot AbsolutePath, path string, patterns []string) (map[RepoRelativeUnixPath]string, error) {
	cmd := exec.Command("git", "ls-files", "-s", "-z", "--")
	cmd.Args = append(cmd.Args, patterns...)
	cmd.Dir = filepath.Join(repoRoot.ToString(), path)

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

func gitStatus(repoRoot AbsolutePath, path string, patterns []string) (map[RepoRelativeUnixPath]status, error) {
	cmd := exec.Command("git", "status", "-u", "-z", "--")
	if len(patterns) == 0 {
		cmd.Args = append(cmd.Args, ".")
	} else {
		cmd.Args = append(cmd.Args, patterns...)
	}
	cmd.Dir = filepath.Join(repoRoot.ToString(), path)

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
