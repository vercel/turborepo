//go:build go || !rust
// +build go !rust

package hashing

import (
	"fmt"
	"os/exec"

	gitignore "github.com/sabhiram/go-gitignore"
	"github.com/vercel/turbo/cli/internal/doublestar"
	"github.com/vercel/turbo/cli/internal/encoding/gitoutput"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

func getPackageFileHashesFromGitIndex(rootPath turbopath.AbsoluteSystemPath, packagePath turbopath.AnchoredSystemPath) (map[turbopath.AnchoredUnixPath]string, error) {
	var result map[turbopath.AnchoredUnixPath]string
	absolutePackagePath := packagePath.RestoreAnchor(rootPath)

	// Get the state of the git index.
	gitLsTreeOutput, err := gitLsTree(absolutePackagePath)
	if err != nil {
		return nil, fmt.Errorf("could not get git hashes for files in package %s: %w", packagePath, err)
	}
	result = gitLsTreeOutput

	// Update the with the state of the working directory.
	// The paths returned from this call are anchored at the package directory
	gitStatusOutput, err := gitStatus(absolutePackagePath)
	if err != nil {
		return nil, fmt.Errorf("Could not get git hashes from git status: %v", err)
	}

	// Review status output to identify the delta.
	var filesToHash []turbopath.AnchoredSystemPath
	for filePath, status := range gitStatusOutput {
		if status.isDelete() {
			delete(result, filePath)
		} else {
			filesToHash = append(filesToHash, filePath.ToSystemPath())
		}
	}

	// Get the hashes for any modified files in the working directory.
	hashes, err := GetHashesForFiles(absolutePackagePath, filesToHash)
	if err != nil {
		return nil, err
	}

	// Zip up file paths and hashes together
	for filePath, hash := range hashes {
		result[filePath] = hash
	}

	return result, nil
}

// gitStatus returns a map of paths to their `git` status code. This can be used to identify what should
// be done with files that do not currently match what is in the index.
//
// Note: `git status -z`'s relative path results are relative to the repository's location.
// We need to calculate where the repository's location is in order to determine what the full path is
// before we can return those paths relative to the calling directory, normalizing to the behavior of
// `ls-files` and `ls-tree`.
func gitStatus(rootPath turbopath.AbsoluteSystemPath) (map[turbopath.AnchoredUnixPath]statusCode, error) {
	cmd := exec.Command(
		"git",               // Using `git` from $PATH,
		"status",            // tell me about the status of the working tree,
		"--untracked-files", // including information about untracked files,
		"--no-renames",      // do not detect renames,
		"-z",                // with each file path relative to the repository root and \000-terminated,
		"--",                // and any additional argument you see is a path, promise.
	)
	cmd.Args = append(cmd.Args, ".") // Operate in the current directory instead of the root of the working tree.
	cmd.Dir = rootPath.ToString()    // Include files only from this directory.

	entries, err := runGitCommand(cmd, "status", gitoutput.NewStatusReader)
	if err != nil {
		return nil, err
	}

	output := make(map[turbopath.AnchoredUnixPath]statusCode, len(entries))
	convertedRootPath := turbopath.AbsoluteSystemPathFromUpstream(rootPath.ToString())

	traversePath, err := memoizedGetTraversePath(convertedRootPath)
	if err != nil {
		return nil, err
	}

	for _, entry := range entries {
		statusEntry := gitoutput.StatusEntry(entry)
		// Anchored at repository.
		pathFromStatus := turbopath.AnchoredUnixPathFromUpstream(statusEntry.GetField(gitoutput.Path))
		var outputPath turbopath.AnchoredUnixPath

		if len(traversePath) > 0 {
			repositoryPath := convertedRootPath.Join(traversePath.ToSystemPath())
			fileFullPath := pathFromStatus.ToSystemPath().RestoreAnchor(repositoryPath)

			relativePath, err := fileFullPath.RelativeTo(convertedRootPath)
			if err != nil {
				return nil, err
			}

			outputPath = relativePath.ToUnixPath()
		} else {
			outputPath = pathFromStatus
		}

		output[outputPath] = statusCode{x: statusEntry.GetField(gitoutput.StatusX), y: statusEntry.GetField(gitoutput.StatusY)}
	}

	return output, nil
}

func safeCompileIgnoreFile(filepath turbopath.AbsoluteSystemPath) (*gitignore.GitIgnore, error) {
	if filepath.FileExists() {
		return gitignore.CompileIgnoreFile(filepath.ToString())
	}
	// no op
	return gitignore.CompileIgnoreLines([]string{}...), nil
}

func getPackageFileHashesFromProcessingGitIgnore(rootPath turbopath.AbsoluteSystemPath, packagePath turbopath.AnchoredSystemPath, inputs []string) (map[turbopath.AnchoredUnixPath]string, error) {
	result := make(map[turbopath.AnchoredUnixPath]string)
	absolutePackagePath := packagePath.RestoreAnchor(rootPath)

	// Instead of implementing all gitignore properly, we hack it. We only respect .gitignore in the root and in
	// the directory of a package.
	ignore, err := safeCompileIgnoreFile(rootPath.UntypedJoin(".gitignore"))
	if err != nil {
		return nil, err
	}

	ignorePkg, err := safeCompileIgnoreFile(absolutePackagePath.UntypedJoin(".gitignore"))
	if err != nil {
		return nil, err
	}

	includePattern := ""
	excludePattern := ""
	if len(inputs) > 0 {
		var includePatterns []string
		var excludePatterns []string
		for _, pattern := range inputs {
			if len(pattern) > 0 && pattern[0] == '!' {
				excludePatterns = append(excludePatterns, absolutePackagePath.UntypedJoin(pattern[1:]).ToString())
			} else {
				includePatterns = append(includePatterns, absolutePackagePath.UntypedJoin(pattern).ToString())
			}
		}
		if len(includePatterns) > 0 {
			includePattern = "{" + strings.Join(includePatterns, ",") + "}"
		}
		if len(excludePatterns) > 0 {
			excludePattern = "{" + strings.Join(excludePatterns, ",") + "}"
		}
	}

	err = fs.Walk(absolutePackagePath.ToStringDuringMigration(), func(name string, isDir bool) error {
		convertedName := turbopath.AbsoluteSystemPathFromUpstream(name)
		rootMatch := ignore.MatchesPath(convertedName.ToString())
		otherMatch := ignorePkg.MatchesPath(convertedName.ToString())
		if !rootMatch && !otherMatch {
			if !isDir {
				if includePattern != "" {
					val, err := doublestar.PathMatch(includePattern, convertedName.ToString())
					if err != nil {
						return err
					}
					if !val {
						return nil
					}
				}
				if excludePattern != "" {
					val, err := doublestar.PathMatch(excludePattern, convertedName.ToString())
					if err != nil {
						return err
					}
					if val {
						return nil
					}
				}
				hash, err := fs.GitLikeHashFile(convertedName)
				if err != nil {
					return fmt.Errorf("could not hash file %v. \n%w", convertedName.ToString(), err)
				}

				relativePath, err := convertedName.RelativeTo(absolutePackagePath)
				if err != nil {
					return fmt.Errorf("File path cannot be made relative: %w", err)
				}
				result[relativePath.ToUnixPath()] = hash
			}
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	return result, nil
}
