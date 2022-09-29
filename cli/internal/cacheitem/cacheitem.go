// Package cacheitem is an abstraction over the creation and restoration of a cache
package cacheitem

import (
	"archive/tar"
	"compress/gzip"
	"crypto/sha512"
	"errors"
	"hash"
	"io"
	"os"
	"reflect"

	"github.com/vercel/turborepo/cli/internal/turbopath"
)

var (
	errMissingSymlinkTarget = errors.New("symlink restoration is delayed")
	errCycleDetected        = errors.New("links in the cache are cyclic")
	errTraversal            = errors.New("tar attempts to write outside of directory")
	errNameMalformed        = errors.New("file name is malformed")
	errNameWindowsUnsafe    = errors.New("file name is not Windows-safe")
	errUnsupportedFileType  = errors.New("attempted to restore unsupported file type")
	errWriteAfterClose      = errors.New("write after close")
)

// CacheItem is a `tar` utility with a little bit extra.
type CacheItem struct {
	// Path is the location on disk for the CacheItem.
	Path turbopath.AbsoluteSystemPath
	// Anchor is the position on disk at which the CacheItem will be restored.
	Anchor turbopath.AbsoluteSystemPath

	// For creation.
	sha    hash.Hash
	tw     *tar.Writer
	gzw    *gzip.Writer
	handle *os.File

	// sticky error.
	err error
}

// Close any open pipes
func (ci *CacheItem) Close() error {
	if ci.err == errWriteAfterClose {
		return nil
	}
	if ci.err != nil {
		return ci.err
	}

	// Close from the beginning of the pipe to the end.
	closers := []io.Closer{ci.tw, ci.gzw, ci.handle}

	for _, closer := range closers {
		// Skip the things which may not exist in this particular instance.
		if reflect.ValueOf(closer).IsZero() {
			continue
		}
		if err := closer.Close(); err != nil {
			return err
		}
	}

	// Pencils down. No more writing.
	ci.err = errWriteAfterClose

	return nil
}

// GetSha returns the SHA-512 hash for the CacheItem.
func (ci *CacheItem) GetSha() ([]byte, error) {
	if ci.sha != nil {
		return ci.sha.Sum(nil), nil
	}

	sha := sha512.New()
	if _, err := io.Copy(sha, ci.handle); err != nil {
		return nil, err
	}

	// Don't mutate the sha until it will return the correct value.
	ci.sha = sha

	return ci.sha.Sum(nil), nil
}
