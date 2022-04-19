package fs

import (
	"bytes"
	"fmt"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"

	"github.com/pkg/errors"
)

// Predefine []byte variables to avoid runtime allocations.
var (
	escapedSlash = []byte(`\\`)
	regularSlash = []byte(`\`)
	escapedTab   = []byte(`\t`)
	regularTab   = []byte("\t")
)

// PackageDepsOptions are parameters for getting git hashes for a filesystem
type PackageDepsOptions struct {
	// PackagePath is the folder path to derive the package dependencies from. This is typically the folder
	// containing package.json. If omitted, the default value is the current working directory.
	PackagePath string
	// ExcludedPaths is an optional array of file path exclusions. If a file should be omitted from the list
	// of dependencies, use this to exclude it.
	ExcludedPaths []string
	// GitPath is an optional alternative path to the git installation
	GitPath string

	InputPatterns []string
}

// GetPackageDeps Builds an object containing git hashes for the files under the specified `packagePath` folder.
func GetPackageDeps(p *PackageDepsOptions) (map[string]string, error) {
	// Add all the checked in hashes.
	// TODO(gsoltis): are these platform-dependent paths?
	var result map[string]string
	if len(p.InputPatterns) == 0 {
		gitLsOutput, err := gitLsTree(p.PackagePath, p.GitPath)
		if err != nil {
			return nil, fmt.Errorf("could not get git hashes for files in package %s: %w", p.PackagePath, err)
		}
		result = parseGitLsTree(gitLsOutput)
	} else {
		gitLsOutput, err := gitLsFiles(p.PackagePath, p.GitPath, p.InputPatterns)
		if err != nil {
			return nil, fmt.Errorf("could not get git hashes for file patterns %v in package %s: %w", p.InputPatterns, p.PackagePath, err)
		}
		parsedLines, err := parseGitLsFiles(gitLsOutput)
		if err != nil {
			return nil, err
		}
		result = parsedLines
	}

	if len(p.ExcludedPaths) > 0 {
		for _, p := range p.ExcludedPaths {
			// @todo explore optimization
			delete(result, p)
		}
	}

	// Update the checked in hashes with the current repo status
	gitStatusOutput, err := gitStatus(p.PackagePath, p.GitPath)
	if err != nil {
		return nil, err
	}
	currentlyChangedFiles := parseGitStatus(gitStatusOutput, p.PackagePath)
	var filesToHash []string
	for filename, changeType := range currentlyChangedFiles {
		if changeType == "D" || (len(changeType) == 2 && string(changeType)[1] == []byte("D")[0]) {
			delete(result, filename)
		} else {
			filesToHash = append(filesToHash, filepath.Join(p.PackagePath, filename))
		}
	}
	normalized := make(map[string]string)
	// These paths are platform-dependent, but already relative to the package root
	for platformSpecificPath, hash := range result {
		platformIndependentPath := filepath.ToSlash(platformSpecificPath)
		normalized[platformIndependentPath] = hash
	}
	hashes, err := GetHashableDeps(filesToHash, p.PackagePath)
	if err != nil {
		return nil, err
	}
	for platformIndependentPath, hash := range hashes {
		normalized[platformIndependentPath] = hash
	}
	return normalized, nil
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

// gitHashForFiles a list of files returns a map of with their git hash values. It uses
// git hash-object under the hood.
// Note that filesToHash must have full paths.
func gitHashForFiles(filesToHash []string) (map[string]string, error) {
	changes := make(map[string]string)
	if len(filesToHash) > 0 {
		input := []string{"hash-object"}
		input = append(input, filesToHash...)
		cmd := exec.Command("git", input...)
		// https://blog.kowalczyk.info/article/wOYk/advanced-command-execution-in-go-with-osexec.html
		out, err := cmd.CombinedOutput()
		if err != nil {
			return nil, fmt.Errorf("git hash-object exited with status: %w", err)
		}
		offByOne := strings.Split(string(out), "\n") // there is an extra ""
		hashes := offByOne[:len(offByOne)-1]
		if len(hashes) != len(filesToHash) {
			return nil, fmt.Errorf("passed %v file paths to Git to hash, but received %v hashes", len(filesToHash), len(hashes))
		}
		for i, hash := range hashes {
			filepath := filesToHash[i]
			changes[filepath] = hash
		}
	}

	return changes, nil
}

// UnescapeChars reverses escaped characters.
func UnescapeChars(in []byte) []byte {
	if bytes.ContainsAny(in, "\\\t") {
		return in
	}

	out := bytes.Replace(in, escapedSlash, regularSlash, -1)
	out = bytes.Replace(out, escapedTab, regularTab, -1)
	return out
}

// gitLsTree executes "git ls-tree" in a folder
func gitLsTree(path string, gitPath string) (string, error) {

	cmd := exec.Command("git", "ls-tree", "HEAD", "-r")
	cmd.Dir = path
	out, err := cmd.CombinedOutput()
	if err != nil {
		return "", fmt.Errorf("failed to read `git ls-tree`: %w", err)
	}
	return strings.TrimSpace(string(out)), nil
}

func gitLsFiles(path string, gitPath string, patterns []string) (string, error) {
	cmd := exec.Command("git", "ls-files", "-s", "--")
	for _, pattern := range patterns {
		cmd.Args = append(cmd.Args, pattern)
	}
	cmd.Dir = path
	out, err := cmd.CombinedOutput()
	if err != nil {
		return "", fmt.Errorf("failed to read `git ls-tree`: %w", err)
	}
	return strings.TrimSpace(string(out)), nil
}

func parseGitLsTree(output string) map[string]string {
	changes := make(map[string]string)
	if len(output) > 0 {
		// A line is expected to look like:
		// 100644 blob 3451bccdc831cb43d7a70ed8e628dcf9c7f888c8    src/typings/tsd.d.ts
		// 160000 commit c5880bf5b0c6c1f2e2c43c95beeb8f0a808e8bac  rushstack
		gitRex := regexp.MustCompile(`([0-9]{6})\s(blob|commit)\s([a-f0-9]{40})\s*(.*)`)
		outputLines := strings.Split(output, "\n")

		for _, line := range outputLines {
			if len(line) > 0 {
				matches := gitRex.MatchString(line)
				if matches {
					// this looks like this
					// [["160000 commit c5880bf5b0c6c1f2e2c43c95beeb8f0a808e8bac  rushstack" "160000" "commit" "c5880bf5b0c6c1f2e2c43c95beeb8f0a808e8bac" "rushstack"]]
					match := gitRex.FindAllStringSubmatch(line, -1)
					if len(match[0][3]) > 0 && len(match[0][4]) > 0 {
						hash := match[0][3]
						filename := parseGitFilename(match[0][4])
						changes[filename] = hash
					}
					// @todo error
				}
			}
		}
	}
	return changes
}

func parseGitLsFiles(output string) (map[string]string, error) {
	changes := make(map[string]string)
	if len(output) > 0 {
		// A line is expected to look like:
		// 100644 3451bccdc831cb43d7a70ed8e628dcf9c7f888c8 0   src/typings/tsd.d.ts
		// 160000 c5880bf5b0c6c1f2e2c43c95beeb8f0a808e8bac 0   rushstack
		gitRex := regexp.MustCompile(`[0-9]{6}\s([a-f0-9]{40})\s[0-3]\s*(.+)`)
		outputLines := strings.Split(output, "\n")

		for _, line := range outputLines {
			if len(line) > 0 {
				match := gitRex.FindStringSubmatch(line)
				// we found matches, and the slice has three parts:
				// 0 - the whole string
				// 1 - the hash
				// 2 - the filename
				if match != nil && len(match) == 3 {
					hash := match[1]
					filename := parseGitFilename(match[2])
					changes[filename] = hash
				} else {
					return nil, fmt.Errorf("failed to parse git ls-files output line %v", line)
				}
			}
		}
	}
	return changes, nil
}

// Couldn't figure out how to deal with special characters. Skipping for now.
// @todo see https://github.com/microsoft/rushstack/blob/925ad8c9e22997c1edf5fe38c53fa618e8180f70/libraries/package-deps-hash/src/getPackageDeps.ts#L19
func parseGitFilename(filename string) string {
	// If there are no double-quotes around the string, then there are no escaped characters
	// to decode, so just return
	dubQuoteRegex := regexp.MustCompile(`^".+"$`)
	if !dubQuoteRegex.MatchString(filename) {
		return filename
	}
	// hack??/
	return string(UnescapeChars([]byte(filename)))

	// @todo special character support
	// what we really need to do is to convert this into golang
	// it seems that solution exists inside of "regexp" module
	// either in "replaceAll" or in "doExecute"
	// in the meantime, we do not support special characters in filenames or quotes
	// // Need to hex encode '%' since we will be decoding the converted octal values from hex
	// filename = filename.replace(/%/g, '%25');
	// // Replace all instances of octal literals with percent-encoded hex (ex. '\347\275\221' -> '%E7%BD%91').
	// // This is done because the octal literals represent UTF-8 bytes, and by converting them to percent-encoded
	// // hex, we can use decodeURIComponent to get the Unicode chars.
	// filename = filename.replace(/(?:\\(\d{1,3}))/g, (match, ...[octalValue, index, source]) => {
	//   // We need to make sure that the backslash is intended to escape the octal value. To do this, walk
	//   // backwards from the match to ensure that it's already escaped.
	//   const trailingBackslashes: RegExpMatchArray | null = (source as string)
	//     .slice(0, index as number)
	//     .match(/\\*$/);
	//   return trailingBackslashes && trailingBackslashes.length > 0 && trailingBackslashes[0].length % 2 === 0
	//     ? `%${parseInt(octalValue, 8).toString(16)}`
	//     : match;
	// });

	// // Finally, decode the filename and unescape the escaped UTF-8 chars
	// return JSON.parse(decodeURIComponent(filename));

}

// gitStatus executes "git status" in a folder
func gitStatus(path string, gitPath string) (string, error) {
	// log.Printf("[TRACE] gitStatus start")
	// defer log.Printf("[TRACE] gitStatus end")
	p := "git"
	if len(gitPath) > 0 {
		p = gitPath
	}
	cmd := exec.Command(p, "status", "-s", "-u", ".")
	cmd.Dir = path
	out, err := cmd.CombinedOutput()
	if err != nil {
		return "", fmt.Errorf("failed to read git status: %w", err)
	}
	// log.Printf("[TRACE] gitStatus result: %v", strings.TrimSpace(string(out)))
	return strings.TrimSpace(string(out)), nil
}

func parseGitStatus(output string, PackagePath string) map[string]string {
	// log.Printf("[TRACE] parseGitStatus start")
	// defer log.Printf("[TRACE] parseGitStatus end")
	changes := make(map[string]string)

	// Typically, output will look something like:
	// M temp_modules/rush-package-deps-hash/package.json
	// D package-deps-hash/src/index.ts

	// If there was an issue with `git ls-tree`, or there are no current changes, processOutputBlocks[1]
	// will be empty or undefined
	if len(output) == 0 {
		// log.Printf("[TRACE] parseGitStatus result: no git changes")
		return changes
	}
	// log.Printf("[TRACE] parseGitStatus result: found git changes")
	gitRex := regexp.MustCompile(`("(\\"|[^"])+")|(\S+\s*)`)
	// Note: The output of git hash-object uses \n newlines regardless of OS.
	outputLines := strings.Split(output, "\n")

	for _, line := range outputLines {
		if len(line) > 0 {
			matches := gitRex.MatchString(line)
			if matches {
				// changeType is in the format of "XY" where "X" is the status of the file in the index and "Y" is the status of
				// the file in the working tree. Some example statuses:
				//   - 'D' == deletion
				//   - 'M' == modification
				//   - 'A' == addition
				//   - '??' == untracked
				//   - 'R' == rename
				//   - 'RM' == rename with modifications
				//   - '[MARC]D' == deleted in work tree
				// Full list of examples: https://git-scm.com/docs/git-status#_short_format

				// Lloks like this
				//[["?? " "" "" "?? "] ["package_deps_hash_test.go" "" "" "package_deps_hash_test.go"]]
				match := gitRex.FindAllStringSubmatch(line, -1)
				if len(match[0]) > 1 {
					changeType := match[0][0]
					fileNameMatches := match[1][1:]
					// log.Printf("match: %q", match)
					// log.Printf("change: %v", strings.TrimRight(changeType, " "))

					// We always care about the last filename in the filenames array. In the case of non-rename changes,
					// the filenames array only contains one file, so we can join all segments that were split on spaces.
					// In the case of rename changes, the last item in the array is the path to the file in the working tree,
					// which is the only one that we care about. It is also surrounded by double-quotes if spaces are
					// included, so no need to worry about joining different segments
					lastFileName := strings.Join(fileNameMatches, "")
					// looks like this
					// [["R  " "" "" "R  "] ["turbo.config.js " "" "" "turbo.config.js "] ["-> " "" "" "-> "] ["turboooz.config.js" "" "" "turboooz.config.js"]]
					if strings.HasPrefix(changeType, "R") {
						lastFileName = strings.Join(match[len(match)-1][1:], "")
					}
					lastFileName = parseGitFilename(lastFileName)
					// log.Printf(lastFileName)
					changes[lastFileName] = strings.TrimRight(changeType, " ")
				}
			}
		}
	}
	return changes
}
