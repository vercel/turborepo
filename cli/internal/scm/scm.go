// Package scm abstracts operations on various tools like git
// Currently, only git is supported.
package scm

import (
	"fmt"
	"path/filepath"
	"github.com/vercel/turborepo/cli/internal/fs"
)

// An SCM represents an SCM implementation that we can ask for various things.
type SCM interface {
	// DescribeIdentifier returns the string that is a "human-readable" identifier of the given revision.
	DescribeIdentifier(revision string) string
	// CurrentRevIdentifier returns the string that specifies what the current revision is.
	CurrentRevIdentifier() string
	// ChangesIn returns a list of modified files in the given diffSpec.
	ChangesIn(diffSpec string, relativeTo string) []string
	// ChangedFiles returns a list of modified files since the given commit, optionally including untracked files.
	ChangedFiles(fromCommit string, includeUntracked bool, relativeTo string) []string
	// IgnoreFile marks a file to be ignored by the SCM.
	IgnoreFiles(gitignore string, files []string) error
	// Remove deletes the given files from the SCM.
	Remove(names []string) error
	// ChangedLines returns the set of lines that have been modified,
	// as a map of filename -> affected line numbers.
	ChangedLines() (map[string][]int, error)
	// Checkout checks out the given revision.
	Checkout(revision string) error
	// CurrentRevDate returns the commit date of the current revision, formatted according to the given format string.
	CurrentRevDate(format string) string
}

// New returns a new SCM instance for this repo root.
// It returns nil if there is no known implementation there.
func New(repoRoot string) SCM {
	if fs.PathExists(filepath.Join(repoRoot, ".git")) {
		return &git{repoRoot: repoRoot}
	}
	return nil
}

// NewFallback returns a new SCM instance for this repo root.
// If there is no known implementation it returns a stub.
func NewFallback(repoRoot string) (SCM, error) {
	if scm := New(repoRoot); scm != nil {
		return scm, nil
	}

	return &stub{}, fmt.Errorf("cannot find a .git folder. Falling back to manual file hashing (which may be slower). If you are running this build in a pruned directory, you can ignore this message. Otherwise, please initialize a git repository in the root of your monorepo")
}
