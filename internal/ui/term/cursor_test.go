//go:build !windows
// +build !windows

// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
package cursor

import (
	"io"
	"strings"
	"testing"

	"github.com/AlecAivazis/survey/v2/terminal"
	"github.com/stretchr/testify/require"
)

func TestEraseLine(t *testing.T) {
	testCases := map[string]struct {
		inWriter    func(writer io.Writer) terminal.FileWriter
		shouldErase bool
	}{
		"should erase a line if the writer is a file": {
			inWriter: func(writer io.Writer) terminal.FileWriter {
				return &fakeFileWriter{w: writer}
			},
			shouldErase: true,
		},
	}

	for name, tc := range testCases {
		t.Run(name, func(t *testing.T) {
			// GIVEN
			buf := new(strings.Builder)

			// WHEN
			EraseLine(tc.inWriter(buf))

			// THEN
			isErased := buf.String() != ""
			require.Equal(t, tc.shouldErase, isErased)
		})
	}
}
