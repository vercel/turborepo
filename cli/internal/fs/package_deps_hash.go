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

	var filesToHash []FilePathInterface
	for filePath, status := range gitStatusOutput {
		if status.x == "D" || status.y == "D" {
			delete(result, filePath)
		} else {
			filesToHash = append(filesToHash, filePath)
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
func GetHashableDeps(rootPath AbsolutePath, files []FilePathInterface) (map[RelativeUnixPath]string, error) {
	return gitHashObject(rootPath, files)
}

// gitHashObject returns a map of paths to their SHA hashes calculated by passing the paths `git hash-object`.
// It will always accept a system path. On Windows it *also* accepts Unix paths.
func gitHashObject(rootPath AbsolutePath, filesToHash []FilePathInterface) (map[RelativeUnixPath]string, error) {
	fileCount := len(filesToHash)
	output := make(map[RelativeUnixPath]string, fileCount)

	if fileCount > 0 {
		cmd := exec.Command(
			"git",           // Using `git` from $PATH,
			"hash-object",   // hash a file,
			"--stdin-paths", // using a list of newline-separated paths from stdin.
		)
		cmd.Dir = rootPath.ToString() // Start at this directory.

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
				_, err := io.WriteString(stdinPipe, strings.ReplaceAll(file.ToString(), "\n", "\\n")+"\n")
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
			var key RelativeUnixPath
			switch filePath := filesToHash[i].(type) {
			case RelativeUnixPath:
				key = filePath
			case RelativeSystemPath:
				key = filePath.ToRelativeUnixPath()
			case AbsoluteUnixPath:
				systemRootPath := StringToSystemPath(rootPath.ToString())
				unixRootPath := systemRootPath.ToUnixPath()
				relativeUnixPath, err := filePath.Rel(unixRootPath)
				if err != nil {
					return nil, err
				}
				key = relativeUnixPath
			case AbsoluteSystemPath:
				systemRootPath := StringToSystemPath(rootPath.ToString())
				relativeSystemPath, err := filePath.Rel(systemRootPath)
				if err != nil {
					return nil, err
				}
				key = relativeSystemPath.ToRelativeUnixPath()
			}
			output[key] = hash
		}
	}

	return output, nil
}

// runGitCommand provides boilerplate command handling for `ls-tree`, `ls-files`, and `status`
func runGitCommand(cmd *exec.Cmd, commandName string, handler func(io.Reader) *gitoutput.Reader) ([][]string, error) {
	out, pipeError := cmd.StdoutPipe()
	if pipeError != nil {
		return nil, fmt.Errorf("failed to read `git %s`: %w", commandName, pipeError)
	}

	startError := cmd.Start()
	if startError != nil {
		return nil, fmt.Errorf("failed to read `git %s`: %w", commandName, startError)
	}

	reader := handler(out)
	entries, readErr := reader.ReadAll()
	if readErr != nil {
		return nil, fmt.Errorf("failed to read `git %s`: %w", commandName, readErr)
	}

	waitErr := cmd.Wait()
	if waitErr != nil {
		return nil, fmt.Errorf("failed to read `git %s`: %w", commandName, waitErr)
	}

	return entries, nil
}

// gitLsTree returns a map of paths to their SHA hashes starting at a particular directory
// that are present in the `git` index at a particular revision.
func gitLsTree(rootPath AbsolutePath, path string) (map[RelativeUnixPath]string, error) {
	cmd := exec.Command(
		"git",     // Using `git` from $PATH,
		"ls-tree", // list the contents of the git index,
		"-r",      // recursively,
		"-z",      // with each file output formatted as \000-terminated strings,
		"HEAD",    // at this specified version.
	)
	cmd.Dir = filepath.Join(rootPath.ToString(), path) // Start at this directory.

	entries, err := runGitCommand(cmd, "ls-tree", gitoutput.NewLSTreeReader)
	if err != nil {
		return nil, err
	}

	output := make(map[RelativeUnixPath]string, len(entries))

	for _, entry := range entries {
		output[UnsafeToRelativeUnixPath(entry[3])] = entry[2]
	}

	return output, nil
}

// gitLsTree returns a map of paths to their SHA hashes starting from a list of patterns relative to a directory
// that are present in the `git` index at a particular revision.
func gitLsFiles(rootPath AbsolutePath, path string, patterns []string) (map[RelativeUnixPath]string, error) {
	cmd := exec.Command(
		"git",      // Using `git` from $PATH,
		"ls-files", // tell me about git index information of some files,
		"--stage",  // including information about the state of the object so that we can get the hashes,
		"-z",       // with each file output formatted as \000-terminated strings,
		"--",       // and any additional argument you see is a path, promise.
	)

	// FIXME: Globbing is accomplished implicitly using shell expansion.
	cmd.Args = append(cmd.Args, patterns...)           // Pass in input patterns as arguments.
	cmd.Dir = filepath.Join(rootPath.ToString(), path) // Start at this directory.

	entries, err := runGitCommand(cmd, "ls-files", gitoutput.NewLSFilesReader)
	if err != nil {
		return nil, err
	}

	output := make(map[RelativeUnixPath]string, len(entries))

	for _, entry := range entries {
		output[UnsafeToRelativeUnixPath(entry[3])] = entry[1]
	}

	return output, nil
}

// statusCode represents the two-letter status code from `git status` with two "named" fields, x & y.
// They have different meanings based upon the actual state of the working tree. Using x & y maps
// to upstream behavior.
type statusCode struct {
	x string
	y string
}

// gitStatus returns a map of paths to their `git` status code. This can be used to identify what should
// be done with files that do not currently match what is in the index.
func gitStatus(rootPath AbsolutePath, path string, patterns []string) (map[RelativeUnixPath]statusCode, error) {
	cmd := exec.Command(
		"git",               // Using `git` from $PATH,
		"status",            // tell me about the status of the working tree,
		"--untracked-files", // including information about untracked files,
		"-z",                // with each file output formatted as \000-terminated strings,
		"--",                // and any additional argument you see is a path, promise.
	)
	if len(patterns) == 0 {
		cmd.Args = append(cmd.Args, ".") // Operate in the current directory instead of the root of the working tree.
	} else {
		// FIXME: Globbing is accomplished implicitly using shell expansion.
		cmd.Args = append(cmd.Args, patterns...) // Pass in input patterns as arguments.
	}
	cmd.Dir = filepath.Join(rootPath.ToString(), path) // Start at this directory.

	entries, err := runGitCommand(cmd, "status", gitoutput.NewStatusReader)
	if err != nil {
		return nil, err
	}

	output := make(map[RelativeUnixPath]statusCode, len(entries))

	for _, entry := range entries {
		output[UnsafeToRelativeUnixPath(entry[2])] = statusCode{x: entry[0], y: entry[1]}
	}

	return output, nil
}
