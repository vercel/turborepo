// Adapted from https://github.com/thought-machine/please/tree/master/src/scm
// Copyright Thought Machine, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
package scm

type stub struct{}

func (s *stub) ChangedFiles(fromCommit string, toCommit string, relativeTo string) ([]string, error) {
	return nil, nil
}

func (s *stub) PreviousContent(fromCommit string, filePath string) ([]byte, error) {
	return nil, nil
}
