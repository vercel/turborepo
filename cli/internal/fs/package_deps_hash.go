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
func GetPackageDeps(rootPath AbsolutePath, p *PackageDepsOptions) (map[RelativeUnixPath]string, error) {
	// Add all the checked in hashes.
	var result map[RelativeUnixPath]string
	if len(p.InputPatterns) == 0 {
		gitLsTreeOutput, err := gitLsTree(rootPath, p.PackagePath)
		if err != nil {
			return nil, fmt.Errorf("could not get git hashes for files in package %s: %w", p.PackagePath, err)
		}
		result = gitLsTreeOutput
	} else {
		gitLsFilesOutput, err := gitLsFiles(rootPath, p.PackagePath, p.InputPatterns)
		if err != nil {
			return nil, fmt.Errorf("could not get git hashes for file patterns %v in package %s: %w", p.InputPatterns, p.PackagePath, err)
		}
		result = gitLsFilesOutput
	}

	// Update the checked in hashes with the current repo status
	gitStatusOutput, err := gitStatus(rootPath, p.PackagePath, p.InputPatterns)
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

	hashes, err := gitHashObject(rootPath, filesToHash)
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
func GetHashableDeps(rootPath AbsolutePath, files []string) (map[RelativeUnixPath]string, error) {
	result, hashError := gitHashObject(rootPath, files)
	if hashError != nil {
		return nil, hashError
	}

	rootPathString := rootPath.ToString()
	relativeResult := make(map[RelativeUnixPath]string)
	for file, hash := range result {
		relativePath, err := filepath.Rel(rootPathString, file.ToString())
		if err != nil {
			return nil, err
		}
		relativeResult[UnsafeToRelativeUnixPath(relativePath)] = hash
	}

	return relativeResult, nil
}

// gitHashObject takes a list of files returns a map of with their git hash values.
// It uses git hash-object under the hood.
// Note that filesToHash must have full paths.
func gitHashObject(rootPath AbsolutePath, filesToHash []string) (map[RelativeUnixPath]string, error) {
	fileCount := len(filesToHash)
	changes := make(map[RelativeUnixPath]string, fileCount)

	if fileCount > 0 {
		cmd := exec.Command("git", "hash-object", "--stdin-paths")
		cmd.Dir = rootPath.ToString()

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
			changes[UnsafeToRelativeUnixPath(filepath)] = hash
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

func gitLsTree(rootPath AbsolutePath, path string) (map[RelativeUnixPath]string, error) {
	cmd := exec.Command("git", "ls-tree", "-r", "-z", "HEAD")
	cmd.Dir = filepath.Join(rootPath.ToString(), path)

	entries, err := runGitCommand(cmd, "ls-tree", gitoutput.NewLSTreeReader)
	if err != nil {
		return nil, err
	}

	changes := make(map[RelativeUnixPath]string, len(entries))

	for _, entry := range entries {
		changes[UnsafeToRelativeUnixPath(entry[3])] = entry[2]
	}

	return changes, nil
}

func gitLsFiles(rootPath AbsolutePath, path string, patterns []string) (map[RelativeUnixPath]string, error) {
	cmd := exec.Command("git", "ls-files", "-s", "-z", "--")
	cmd.Args = append(cmd.Args, patterns...)
	cmd.Dir = filepath.Join(rootPath.ToString(), path)

	entries, err := runGitCommand(cmd, "ls-files", gitoutput.NewLSFilesReader)
	if err != nil {
		return nil, err
	}

	changes := make(map[RelativeUnixPath]string, len(entries))

	for _, entry := range entries {
		changes[UnsafeToRelativeUnixPath(entry[3])] = entry[1]
	}

	return changes, nil
}

type status struct {
	x string
	y string
}

func gitStatus(rootPath AbsolutePath, path string, patterns []string) (map[RelativeUnixPath]status, error) {
	cmd := exec.Command("git", "status", "-u", "-z", "--")
	if len(patterns) == 0 {
		cmd.Args = append(cmd.Args, ".")
	} else {
		cmd.Args = append(cmd.Args, patterns...)
	}
	cmd.Dir = filepath.Join(rootPath.ToString(), path)

	entries, err := runGitCommand(cmd, "status", gitoutput.NewStatusReader)
	if err != nil {
		return nil, err
	}

	changes := make(map[RelativeUnixPath]status, len(entries))

	for _, entry := range entries {
		changes[UnsafeToRelativeUnixPath(entry[2])] = status{x: entry[0], y: entry[1]}
	}

	return changes, nil
}
