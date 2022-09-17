package cacheitem

import (
	"archive/tar"
	"compress/gzip"
	"crypto/sha512"
	"io"
	"io/fs"
	"os"

	"github.com/moby/sys/sequential"
	"github.com/vercel/turborepo/cli/internal/turbopath"
)

// Create makes a new CacheItem at the specified path.
func Create(path turbopath.AbsoluteSystemPath) (*CacheItem, error) {
	handle, err := os.Create(path.ToString())
	if err != nil {
		return nil, err
	}

	return &CacheItem{
		Path:   path,
		handle: handle,
	}, nil
}

// init prepares the CacheItem for writing.
// Wires all the writers end-to-end:
// tar.Writer -> gzip.Writer -> io.MultiWriter -> (file & sha)
func (ci *CacheItem) init() {
	ci.once.Do(func() {
		sha := sha512.New()
		mw := io.MultiWriter(sha, ci.handle)
		gzw := gzip.NewWriter(mw)
		tw := tar.NewWriter(gzw)

		ci.tw = tw
		ci.gzw = gzw
		ci.sha = sha
	})
}

// AddMetadata adds a file which is not part of the cache to the `tar`.
// The contents of this file should not contain user input.
func (ci *CacheItem) AddMetadata(anchor turbopath.AbsoluteSystemPath, path turbopath.AnchoredSystemPath) error {
	ci.init()
	return ci.addFile(turbopath.AnchoredSystemPath("metadata"), anchor, path)
}

// AddFile adds a user-cached item to the tar.
func (ci *CacheItem) AddFile(anchor turbopath.AbsoluteSystemPath, path turbopath.AnchoredSystemPath) error {
	ci.init()
	return ci.addFile(turbopath.AnchoredSystemPath("cache"), anchor, path)
}

// addFile is the actual interface to the tar file.
func (ci *CacheItem) addFile(cacheAnchor turbopath.AnchoredSystemPath, fsAnchor turbopath.AbsoluteSystemPath, filePath turbopath.AnchoredSystemPath) error {
	// Calculate the fully-qualified path to the file to read it.
	sourcePath := filePath.RestoreAnchor(fsAnchor)

	// We grab the FileInfo which tar.FileInfoHeader accepts.
	fileInfo, lstatErr := os.Lstat(sourcePath.ToString())
	if lstatErr != nil {
		return lstatErr
	}

	// Determine if we need to populate the additional link argument to tar.FileInfoHeader.
	var link string
	if fileInfo.Mode()&fs.ModeSymlink != 0 {
		linkTarget, readlinkErr := os.Readlink(sourcePath.ToString())
		if readlinkErr != nil {
			return readlinkErr
		}
		link = linkTarget
	}

	// Reanchor the file within the cache and normalize.
	cacheDestinationName := filePath.Move(cacheAnchor).ToUnixPath()

	// Generate the the header.
	// We do not use header generation from stdlib because it can throw an error.
	header, headerErr := FileInfoHeader(cacheDestinationName, fileInfo, link)
	if headerErr != nil {
		return headerErr
	}

	// Always write the header.
	if err := ci.tw.WriteHeader(header); err != nil {
		return err
	}

	// If there is a body to be written, do so.
	if header.Typeflag == tar.TypeReg && header.Size > 0 {
		// Windows has a distinct "sequential read" opening mode.
		// We use a library that will switch to this mode for Windows.
		sourceFile, sourceErr := sequential.Open(sourcePath.ToString())
		defer func() { _ = sourceFile.Close() }()
		if sourceErr != nil {
			return sourceErr
		}

		if _, err := io.Copy(ci.tw, sourceFile); err != nil {
			return err
		}
	}

	return nil
}
