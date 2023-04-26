//go:build go || !rust
// +build go !rust

// Package scm abstracts operations on various tools like git
// Currently, only git is supported.
//
// Adapted from https://github.com/thought-machine/please/tree/master/src/scm
// Copyright Thought Machine, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
package scm

import (
	"fmt"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/pkg/errors"
)

// git implements operations on a git repository.
type git struct {
	repoRoot turbopath.AbsoluteSystemPath
}

// ChangedFiles returns a list of modified files since the given commit, optionally including untracked files.
func (g *git) ChangedFiles(fromCommit string, toCommit string, relativeTo string) ([]string, error) {
	if relativeTo == "" {
		relativeTo = g.repoRoot.ToString()
	}
	relSuffix := []string{"--", relativeTo}
	command := []string{"diff", "--name-only", toCommit}

	out, err := exec.Command("git", append(command, relSuffix...)...).CombinedOutput()
	if err != nil {
		return nil, errors.Wrapf(err, "finding changes relative to %v", relativeTo)
	}
	files := strings.Split(string(out), "\n")

	if fromCommit != "" {
		// Grab the diff from the merge-base to HEAD using ... syntax.  This ensures we have just
		// the changes that have occurred on the current branch.
		command = []string{"diff", "--name-only", fromCommit + "..." + toCommit}
		out, err = exec.Command("git", append(command, relSuffix...)...).CombinedOutput()
		if err != nil {
			// Check if we can provide a better error message for non-existent commits.
			// If we error on the check or can't find it, fall back to whatever error git
			// reported.
			if exists, err := commitExists(fromCommit); err == nil && !exists {
				return nil, fmt.Errorf("commit %v does not exist", fromCommit)
			}
			return nil, errors.Wrapf(err, "git comparing with %v", fromCommit)
		}
		committedChanges := strings.Split(string(out), "\n")
		files = append(files, committedChanges...)
	}
	command = []string{"ls-files", "--other", "--exclude-standard"}
	out, err = exec.Command("git", append(command, relSuffix...)...).CombinedOutput()
	if err != nil {
		return nil, errors.Wrap(err, "finding untracked files")
	}
	untracked := strings.Split(string(out), "\n")
	files = append(files, untracked...)
	// git will report changed files relative to the worktree: re-relativize to relativeTo
	normalized := make([]string, 0)
	for _, f := range files {
		if f == "" {
			continue
		}
		normalizedFile, err := g.fixGitRelativePath(strings.TrimSpace(f), relativeTo)
		if err != nil {
			return nil, err
		}
		normalized = append(normalized, normalizedFile)
	}
	return normalized, nil
}

func (g *git) PreviousContent(fromCommit string, filePath string) ([]byte, error) {
	if fromCommit == "" {
		return nil, fmt.Errorf("Need commit sha to inspect file contents")
	}

	out, err := exec.Command("git", "show", fmt.Sprintf("%s:%s", fromCommit, filePath)).CombinedOutput()
	if err != nil {
		return nil, errors.Wrapf(err, "unable to get contents of %s", filePath)
	}

	return out, nil
}

func commitExists(commit string) (bool, error) {
	err := exec.Command("git", "cat-file", "-t", commit).Run()
	if err != nil {
		exitErr := &exec.ExitError{}
		if errors.As(err, &exitErr) && exitErr.ExitCode() == 128 {
			return false, nil
		}
		return false, err
	}
	return true, nil
}

func (g *git) fixGitRelativePath(worktreePath, relativeTo string) (string, error) {
	p, err := filepath.Rel(relativeTo, filepath.Join(g.repoRoot, worktreePath))
	if err != nil {
		return "", errors.Wrapf(err, "unable to determine relative path for %s and %s", g.repoRoot, relativeTo)
	}
	return p, nil
}
