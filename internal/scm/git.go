// Package scm abstracts operations on various tools like git
// Currently, only git is supported.
package scm

import (
	"bufio"
	"fmt"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/sourcegraph/go-diff/diff"
)

// git implements operations on a git repository.
type git struct {
	repoRoot string
}

// DescribeIdentifier returns the string that is a "human-readable" identifier of the given revision.
func (g *git) DescribeIdentifier(revision string) string {
	out, err := exec.Command("git", "describe", "--always", revision).CombinedOutput()
	if err != nil {
		log.Fatalf("Failed to read %s: %s", revision, err)
	}
	return strings.TrimSpace(string(out))
}

// CurrentRevIdentifier returns the string that specifies what the current revision is.
func (g *git) CurrentRevIdentifier() string {
	out, err := exec.Command("git", "rev-parse", "HEAD").CombinedOutput()
	if err != nil {
		log.Fatalf("Failed to read HEAD: %s", err)
	}
	return strings.TrimSpace(string(out))
}

// ChangesIn returns a list of modified files in the given diffSpec.
func (g *git) ChangesIn(diffSpec string, relativeTo string) []string {
	if relativeTo == "" {
		relativeTo = g.repoRoot
	}
	files := make([]string, 0)
	command := []string{"diff-tree", "--no-commit-id", "--name-only", "-r", diffSpec}
	out, err := exec.Command("git", command...).CombinedOutput()
	if err != nil {
		log.Fatalf("unable to determine changes: %s", err)
	}
	output := strings.Split(string(out), "\n")
	for _, o := range output {
		files = append(files, g.fixGitRelativePath(strings.TrimSpace(o), relativeTo))
	}
	return files
}

// ChangedFiles returns a list of modified files since the given commit, optionally including untracked files.
func (g *git) ChangedFiles(fromCommit string, includeUntracked bool, relativeTo string) []string {
	if relativeTo == "" {
		relativeTo = g.repoRoot
	}
	relSuffix := []string{"--", relativeTo}
	command := []string{"diff", "--name-only", "HEAD"}

	out, err := exec.Command("git", append(command, relSuffix...)...).CombinedOutput()
	if err != nil {
		log.Fatalf("unable to find changes: %s", err)
	}
	files := strings.Split(string(out), "\n")

	if fromCommit != "" {
		// Grab the diff from the merge-base to HEAD using ... syntax.  This ensures we have just
		// the changes that have occurred on the current branch.
		command = []string{"diff", "--name-only", fromCommit + "...HEAD"}
		out, err = exec.Command("git", append(command, relSuffix...)...).CombinedOutput()
		if err != nil {
			log.Fatalf("unable to check diff vs. %s: %s", fromCommit, err)
		}
		committedChanges := strings.Split(string(out), "\n")
		files = append(files, committedChanges...)
	}
	if includeUntracked {
		command = []string{"ls-files", "--other", "--exclude-standard"}
		out, err = exec.Command("git", append(command, relSuffix...)...).CombinedOutput()
		if err != nil {
			log.Fatalf("unable to determine untracked files: %s", err)
		}
		untracked := strings.Split(string(out), "\n")
		files = append(files, untracked...)
	}
	// git will report changed files relative to the worktree: re-relativize to relativeTo
	normalized := make([]string, 0)
	for _, f := range files {
		normalized = append(normalized, g.fixGitRelativePath(strings.TrimSpace(f), relativeTo))
	}
	return normalized
}

func (g *git) fixGitRelativePath(worktreePath, relativeTo string) string {
	p, err := filepath.Rel(relativeTo, filepath.Join(g.repoRoot, worktreePath))
	if err != nil {
		log.Fatalf("unable to determine relative path for %s and %s", g.repoRoot, relativeTo)
	}
	return p
}

const pleaseDoNotEdit = "# Entries below this point are managed by Turbo (DO NOT EDIT)"

var defaultIgnoredFiles = []string{"plz-out", ".plzconfig.local"}

func readUserEntries(file string) ([]string, error) {
	f, err := os.Open(file)
	if err != nil && !os.IsNotExist(err) {
		return nil, err
	}
	defer f.Close()

	scanner := bufio.NewScanner(f)

	var userEntires []string
	for scanner.Scan() {
		line := scanner.Text()
		if strings.TrimSpace(line) == pleaseDoNotEdit {
			return userEntires, nil
		}
		userEntires = append(userEntires, line)
	}
	return userEntires, nil
}

func (g *git) IgnoreFiles(gitignore string, files []string) error {
	// If we're generating the ignore in the root of the project, we should ignore some Please stuff too
	if gitignore == ".gitignore" {
		files = append(defaultIgnoredFiles, files...)
	}

	p := filepath.Join(g.repoRoot, gitignore)

	userEntries, err := readUserEntries(p)
	if err != nil {
		return err
	}

	lines := userEntries
	if len(lines) != 0 && lines[len(lines)-1] != "" {
		lines = append(lines, "")
	}
	lines = append(lines, pleaseDoNotEdit)
	lines = append(lines, files...)

	if err := os.RemoveAll(p); err != nil && err != os.ErrNotExist {
		return err
	}

	file, err := os.Create(p)
	if err != nil {
		return err
	}
	defer file.Close()

	for _, line := range lines {
		if _, err := fmt.Fprintln(file, line); err != nil {
			return err
		}
	}
	return nil
}

func (g *git) Remove(names []string) error {
	cmd := exec.Command("git", append([]string{"rm", "-q"}, names...)...)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("git rm failed: %w %s", err, out)
	}
	return nil
}

func (g *git) ChangedLines() (map[string][]int, error) {
	cmd := exec.Command("git", "diff", "origin/master", "--unified=0", "--no-color", "--no-ext-diff")
	out, err := cmd.CombinedOutput()
	if err != nil {
		return nil, fmt.Errorf("git diff failed: %w", err)
	}
	return g.parseChangedLines(out)
}

func (g *git) parseChangedLines(input []byte) (map[string][]int, error) {
	m := map[string][]int{}
	fds, err := diff.ParseMultiFileDiff(input)
	for _, fd := range fds {
		m[strings.TrimPrefix(fd.NewName, "b/")] = g.parseHunks(fd.Hunks)
	}
	return m, err
}

func (g *git) parseHunks(hunks []*diff.Hunk) []int {
	ret := []int{}
	for _, hunk := range hunks {
		for i := 0; i < int(hunk.NewLines); i++ {
			ret = append(ret, int(hunk.NewStartLine)+i)
		}
	}
	return ret
}

func (g *git) Checkout(revision string) error {
	if out, err := exec.Command("git", "checkout", revision).CombinedOutput(); err != nil {
		return fmt.Errorf("git checkout of %s failed: %w\n%s", revision, err, out)
	}
	return nil
}

func (g *git) CurrentRevDate(format string) string {
	out, err := exec.Command("git", "show", "-s", "--format=%ct").CombinedOutput()
	if err != nil {
		return "Unknown"
	}
	timestamp, err := strconv.ParseInt(strings.TrimSpace(string(out)), 10, 64)
	if err != nil {
		return err.Error()
	}
	t := time.Unix(timestamp, 0)
	return t.Format(format)
}
