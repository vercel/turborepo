//go:build !windows
// +build !windows

// Adapted from https://github.com/moby/moby/blob/924edb948c2731df3b77697a8fcc85da3f6eef57/pkg/archive/archive_unix.go
// Copyright Docker, Inc.
// SPDX-License-Identifier: Apache-2.0

package tarpatch

import (
	"archive/tar"
	"os"
	"syscall"

	"golang.org/x/sys/unix"
)

// chmodTarEntry is used to adjust the file permissions used in tar header based
// on the platform the archival is done.
func chmodTarEntry(perm os.FileMode) os.FileMode {
	return perm // noop for unix as golang APIs provide perm bits correctly
}

// sysStat populates hdr from system-dependent fields of fi without performing
// any OS lookups.
func sysStat(fi os.FileInfo, hdr *tar.Header) error {
	s, ok := fi.Sys().(*syscall.Stat_t)
	if !ok {
		return nil
	}

	hdr.Uid = int(s.Uid)
	hdr.Gid = int(s.Gid)

	if s.Mode&unix.S_IFBLK != 0 ||
		s.Mode&unix.S_IFCHR != 0 {
		hdr.Devmajor = int64(unix.Major(uint64(s.Rdev))) //nolint: unconvert
		hdr.Devminor = int64(unix.Minor(uint64(s.Rdev))) //nolint: unconvert
	}

	return nil
}
