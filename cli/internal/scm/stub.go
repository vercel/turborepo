// Adapted from https://github.com/thought-machine/please/tree/master/src/scm
// Copyright Thought Machine, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
package scm

import "fmt"

type stub struct{}

func (s *stub) DescribeIdentifier(sha string) string {
	return "<unknown>"
}

func (s *stub) CurrentRevIdentifier() string {
	return "<unknown>"
}

func (s *stub) ChangesIn(diffSpec string, relativeTo string) []string {
	return nil
}

func (s *stub) ChangedFiles(fromCommit string, includeUntracked bool, relativeTo string) ([]string, error) {
	return nil, nil
}

func (s *stub) IgnoreFiles(string, []string) error {
	return fmt.Errorf("don't know how to mark files as ignored: unsupported SCM")
}

func (s *stub) Remove(names []string) error {
	return fmt.Errorf("unknown SCM, can't remove files")
}

func (s *stub) ChangedLines() (map[string][]int, error) {
	return nil, fmt.Errorf("unknown SCM, can't calculate changed lines")
}

func (s *stub) Checkout(revision string) error {
	return fmt.Errorf("unknown SCM, can't checkout")
}

func (s *stub) CurrentRevDate(format string) string {
	return "Unknown"
}
