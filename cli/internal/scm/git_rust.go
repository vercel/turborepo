// Package scm abstracts operations on various tools like git
// Currently, only git is supported.
//
// Adapted from https://github.com/thought-machine/please/tree/master/src/scm
// Copyright Thought Machine, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//go:build rust
// +build rust

package scm

import (
	"github.com/vercel/turbo/cli/internal/ffi"
)

// git implements operations on a git repository.
type git struct {
	repoRoot string
}

// ChangedFiles returns a list of modified files since the given commit, optionally including untracked files.
func (g *git) ChangedFiles(fromCommit string, toCommit string, includeUntracked bool, relativeTo string) ([]string, error) {
	return ffi.ChangedFiles(g.repoRoot, fromCommit, toCommit, includeUntracked, relativeTo)
}

func (g *git) PreviousContent(fromCommit string, filePath string) ([]byte, error) {
	return ffi.PreviousContent(g.repoRoot, fromCommit, filePath)
}
