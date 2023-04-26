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
	"fmt"
	"github.com/vercel/turbo/cli/internal/ffi"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// git implements operations on a git repository.
type git struct {
	repoRoot turbopath.AbsoluteSystemPath
}

// ChangedFiles returns a list of modified files since the given commit, optionally including untracked files.
func (g *git) ChangedFiles(fromCommit string, toCommit string, monorepoRoot string) ([]string, error) {
	return ffi.ChangedFiles(g.repoRoot.ToString(), monorepoRoot, fromCommit, toCommit)
}

func (g *git) PreviousContent(fromCommit string, filePath string) ([]byte, error) {
	if fromCommit == "" {
		return nil, fmt.Errorf("Need commit sha to inspect file contents")
	}

	return ffi.PreviousContent(g.repoRoot.ToString(), fromCommit, filePath)
}
