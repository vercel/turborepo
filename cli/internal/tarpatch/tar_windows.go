//go:build windows
// +build windows

// Adapted from https://github.com/moby/moby/blob/924edb948c2731df3b77697a8fcc85da3f6eef57/pkg/archive/archive_windows.go
// Copyright Docker, Inc.
// SPDX-License-Identifier: Apache-2.0

package tarpatch

import (
	"archive/tar"
	"os"
)

// chmodTarEntry is used to adjust the file permissions used in tar header based
// on the platform the archival is done.
func chmodTarEntry(perm os.FileMode) os.FileMode {
	// Remove group- and world-writable bits.
	perm &= 0o755

	// Add the x bit: make everything +x on Windows
	return perm | 0o111
}

func sysStat(fi os.FileInfo, hdr *tar.Header) error {
	return nil
}
