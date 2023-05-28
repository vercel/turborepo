// Package scm abstracts operations on various tools like git
// Currently, only git is supported.
//
// Adapted from https://github.com/thought-machine/please/tree/master/src/scm
// Copyright Thought Machine, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
package scm

import (
	"os/exec"
	"strings"

	"github.com/pkg/errors"

	"github.com/vercel/turbo/cli/internal/turbopath"
)

var ErrFallback = errors.New("cannot find a .git folder. Falling back to manual file hashing (which may be slower). If you are running this build in a pruned directory, you can ignore this message. Otherwise, please initialize a git repository in the root of your monorepo")

// An SCM represents an SCM implementation that we can ask for various things.
type SCM interface {
	// ChangedFiles returns a list of modified files since the given commit, including untracked files
	ChangedFiles(fromCommit string, toCommit string, relativeTo string) ([]string, error)
	// PreviousContent Returns the content of the file at fromCommit
	PreviousContent(fromCommit string, filePath string) ([]byte, error)
}

// newGitSCM returns a new SCM instance for this repo root.
// It returns nil if there is no known implementation there.
func newGitSCM(repoRoot turbopath.AbsoluteSystemPath) SCM {
	if repoRoot.UntypedJoin(".git").Exists() {
		return &git{repoRoot: repoRoot}
	}
	return nil
}

// newFallback returns a new SCM instance for this repo root.
// If there is no known implementation it returns a stub.
func newFallback(repoRoot turbopath.AbsoluteSystemPath) (SCM, error) {
	if scm := newGitSCM(repoRoot); scm != nil {
		return scm, nil
	}

	return &stub{}, ErrFallback
}

// FromInRepo produces an SCM instance, given a path within a
// repository. It does not need to be a git repository, and if
// it is not, the given path is assumed to be the root.
func FromInRepo(repoRoot turbopath.AbsoluteSystemPath) (SCM, error) {
	dotGitDir, err := repoRoot.Findup(".git")
	if err != nil {
		return nil, err
	}
	return newFallback(dotGitDir.Dir())
}

// GetCurrentBranch returns the current branch
func GetCurrentBranch(dir turbopath.AbsoluteSystemPath) string {
	cmd := exec.Command("git", []string{"branch", "--show-current"}...)
	cmd.Dir = dir.ToString()

	out, err := cmd.Output()
	if err != nil {
		return ""
	}
	return strings.TrimRight(string(out), "\n")
}

// GetCurrentSha returns the current SHA
func GetCurrentSha(dir turbopath.AbsoluteSystemPath) string {
	cmd := exec.Command("git", []string{"rev-parse", "HEAD"}...)
	cmd.Dir = dir.ToString()

	out, err := cmd.Output()
	if err != nil {
		return ""
	}
	return strings.TrimRight(string(out), "\n")
}
