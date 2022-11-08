// Adapted from https://github.com/moby/moby/blob/924edb948c2731df3b77697a8fcc85da3f6eef57/pkg/archive/archive.go
// Copyright Docker, Inc.
// SPDX-License-Identifier: Apache-2.0

// Package tarpatch addresses an issue with stdlib throwing an error in some environments.
package tarpatch

import (
	"archive/tar"
	"io/fs"
	"os"
	"strings"
	"time"

	"github.com/vercel/turbo/cli/internal/turbopath"
)

// nosysFileInfo hides the system-dependent info of the wrapped FileInfo to
// prevent tar.FileInfoHeader from introspecting it and potentially calling into
// glibc.
type nosysFileInfo struct {
	os.FileInfo
}

func (fi nosysFileInfo) Sys() interface{} {
	// A Sys value of type *tar.Header is safe as it is system-independent.
	// The tar.FileInfoHeader function copies the fields into the returned
	// header without performing any OS lookups.
	if sys, ok := fi.FileInfo.Sys().(*tar.Header); ok {
		return sys
	}
	return nil
}

// FileInfoHeaderNoLookups creates a partially-populated tar.Header from fi.
//
// Compared to the archive/tar.FileInfoHeader function, this function is safe to
// call from a chrooted process as it does not populate fields which would
// require operating system lookups. It behaves identically to
// tar.FileInfoHeader when fi is a FileInfo value returned from
// tar.Header.FileInfo().
//
// When fi is a FileInfo for a native file, such as returned from os.Stat() and
// os.Lstat(), the returned Header value differs from one returned from
// tar.FileInfoHeader in the following ways. The Uname and Gname fields are not
// set as OS lookups would be required to populate them. The AccessTime and
// ChangeTime fields are not currently set (not yet implemented) although that
// is subject to change. Callers which require the AccessTime or ChangeTime
// fields to be zeroed should explicitly zero them out in the returned Header
// value to avoid any compatibility issues in the future.
func FileInfoHeaderNoLookups(fi fs.FileInfo, link string) (*tar.Header, error) {
	hdr, err := tar.FileInfoHeader(nosysFileInfo{fi}, link)
	if err != nil {
		return nil, err
	}
	return hdr, sysStat(fi, hdr)
}

// FileInfoHeader creates a populated Header from fi.
//
// Compared to the archive/tar package, this function fills in less information
// but is safe to call from a chrooted process. The AccessTime and ChangeTime
// fields are not set in the returned header, ModTime is truncated to one-second
// precision, and the Uname and Gname fields are only set when fi is a FileInfo
// value returned from tar.Header.FileInfo().
func FileInfoHeader(fullPath turbopath.AnchoredUnixPath, fileInfo fs.FileInfo, link string) (*tar.Header, error) {
	hdr, err := FileInfoHeaderNoLookups(fileInfo, link)
	if err != nil {
		return nil, err
	}
	hdr.Format = tar.FormatPAX
	hdr.ModTime = hdr.ModTime.Truncate(time.Second)
	hdr.AccessTime = time.Time{}
	hdr.ChangeTime = time.Time{}
	hdr.Mode = int64(chmodTarEntry(os.FileMode(hdr.Mode)))
	hdr.Name = canonicalTarName(fullPath, fileInfo.IsDir())
	return hdr, nil
}

// canonicalTarName provides a platform-independent and consistent posix-style
// path for files and directories to be archived regardless of the platform.
func canonicalTarName(fullPath turbopath.AnchoredUnixPath, isDir bool) string {
	nameString := fullPath.ToString()
	if isDir {
		// Append '/' if not already present.
		if !strings.HasSuffix(nameString, "/") {
			nameString += "/"
		}
	}

	return nameString
}
