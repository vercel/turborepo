//go:build darwin
// +build darwin

// Adapted from https://github.com/containerd/continuity/blob/b4ca35286886296377de39e6eafd1affae019fc3/driver/lchmod_unix.go
// Copyright The containerd Authors
// SPDX-License-Identifier: Apache-2.0

package turbopath

import (
	"os"

	"golang.org/x/sys/unix"
)

// Lchmod changes the mode of a file not following symlinks.
func (p AbsoluteSystemPath) Lchmod(mode os.FileMode) error {
	err := unix.Fchmodat(unix.AT_FDCWD, p.ToString(), uint32(mode), unix.AT_SYMLINK_NOFOLLOW)
	if err != nil {
		err = &os.PathError{Op: "lchmod", Path: p.ToString(), Err: err}
	}
	return err
}
