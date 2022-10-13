// Package cacheitem is an abstraction over the creation and restoration of a cache
package cacheitem

import (
	"archive/tar"
	"bufio"
	"crypto/sha512"
	"errors"
	"io"
	"os"

	"github.com/vercel/turborepo/cli/internal/turbopath"
)

var (
	errMissingSymlinkTarget = errors.New("symlink restoration is delayed")
	errCycleDetected        = errors.New("links in the cache are cyclic")
	errTraversal            = errors.New("tar attempts to write outside of directory")
	errNameMalformed        = errors.New("file name is malformed")
	errNameWindowsUnsafe    = errors.New("file name is not Windows-safe")
	errUnsupportedFileType  = errors.New("attempted to restore unsupported file type")
)

// CacheItem is a `tar` utility with a little bit extra.
type CacheItem struct {
	// Path is the location on disk for the CacheItem.
	Path turbopath.AbsoluteSystemPath
	// Anchor is the position on disk at which the CacheItem will be restored.
	Anchor turbopath.AbsoluteSystemPath

	// For creation.
	tw         *tar.Writer
	zw         io.WriteCloser
	fileBuffer *bufio.Writer
	handle     *os.File
	compressed bool
}

// Close any open pipes
func (ci *CacheItem) Close() error {
	if ci.tw != nil {
		if err := ci.tw.Close(); err != nil {
			return err
		}
	}

	if ci.zw != nil {
		if err := ci.zw.Close(); err != nil {
			return err
		}
	}

	if ci.fileBuffer != nil {
		if err := ci.fileBuffer.Flush(); err != nil {
			return err
		}
	}

	if ci.handle != nil {
		if err := ci.handle.Close(); err != nil {
			return err
		}
	}

	return nil
}

// GetSha returns the SHA-512 hash for the CacheItem.
func (ci *CacheItem) GetSha() ([]byte, error) {
	sha := sha512.New()
	if _, err := io.Copy(sha, ci.handle); err != nil {
		return nil, err
	}

	return sha.Sum(nil), nil
}
