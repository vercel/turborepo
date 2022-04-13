// Package scm abstracts operations on various tools like git
// Currently, only git is supported.

// Adapted from https://github.com/thought-machine/please/tree/master/src/scm
// Copyright Thought Machine, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
package scm

import (
	"path/filepath"

	"github.com/pkg/errors"

	"github.com/vercel/turborepo/cli/internal/fs"
)

var ErrFallback = errors.New("cannot find a .git folder. Falling back to manual file hashing (which may be slower). If you are running this build in a pruned directory, you can ignore this message. Otherwise, please initialize a git repository in the root of your monorepo")

// An SCM represents an SCM implementation that we can ask for various things.
type SCM interface {
	// ChangedFiles returns a list of modified files since the given commit, optionally including untracked files.*/
	ChangedFiles(fromCommit string, includeUntracked bool, relativeTo string) []string
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

	return &stub{}, ErrFallback
}

func FromInRepo(cwd string) (SCM, error) {
	dotGitDir, err := fs.FindupFrom(".git", cwd)
	if err != nil {
		return nil, err
	}
	return NewFallback(filepath.Dir(dotGitDir))
}
