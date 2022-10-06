//go:build windows
// +build windows

package cacheitem

import (
	"testing"

	"github.com/vercel/turborepo/cli/internal/turbopath"
)

func createFifo(t *testing.T, anchor turbopath.AbsoluteSystemPath, fileDefinition createFileDefinition) error {
	return errUnsupportedFileType
}
