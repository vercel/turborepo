package cacheitem

import (
	"archive/tar"
	"bufio"
	"io"
	"os"
	"strings"
	"time"

	"github.com/DataDog/zstd"

	"github.com/moby/sys/sequential"
	"github.com/vercel/turborepo/cli/internal/tarpatch"
	"github.com/vercel/turborepo/cli/internal/turbopath"
)

// Create makes a new CacheItem at the specified path.
func Create(path turbopath.AbsoluteSystemPath) (*CacheItem, error) {
	handle, err := path.OpenFile(os.O_WRONLY|os.O_CREATE|os.O_TRUNC|os.O_APPEND, 0644)
	if err != nil {
		return nil, err
	}

	cacheItem := &CacheItem{
		Path:       path,
		handle:     handle,
		compressed: strings.HasSuffix(path.ToString(), ".zst"),
	}

	cacheItem.init()
	return cacheItem, nil
}

// init prepares the CacheItem for writing.
// Wires all the writers end-to-end:
// tar.Writer -> zstd.Writer -> fileBuffer -> file
func (ci *CacheItem) init() {
	fileBuffer := bufio.NewWriterSize(ci.handle, 2^20) // Flush to disk in 1mb chunks.

	var tw *tar.Writer
	if ci.compressed {
		zw := zstd.NewWriter(fileBuffer)
		tw = tar.NewWriter(zw)
		ci.zw = zw
	} else {
		tw = tar.NewWriter(fileBuffer)
	}

	ci.tw = tw
	ci.fileBuffer = fileBuffer
}

// AddFile adds a user-cached item to the tar.
func (ci *CacheItem) AddFile(fsAnchor turbopath.AbsoluteSystemPath, filePath turbopath.AnchoredSystemPath) error {
	// Calculate the fully-qualified path to the file to read it.
	sourcePath := filePath.RestoreAnchor(fsAnchor)

	// We grab the FileInfo which tar.FileInfoHeader accepts.
	fileInfo, lstatErr := sourcePath.Lstat()
	if lstatErr != nil {
		return lstatErr
	}

	// Determine if we need to populate the additional link argument to tar.FileInfoHeader.
	var link string
	if fileInfo.Mode()&os.ModeSymlink != 0 {
		linkTarget, readlinkErr := sourcePath.Readlink()
		if readlinkErr != nil {
			return readlinkErr
		}
		link = linkTarget
	}

	// Normalize the path within the cache.
	cacheDestinationName := filePath.ToUnixPath()

	// Generate the the header.
	// We do not use header generation from stdlib because it can throw an error.
	header, headerErr := tarpatch.FileInfoHeader(cacheDestinationName, fileInfo, link)
	if headerErr != nil {
		return headerErr
	}

	// Throw an error if trying to create a cache that contains a type we don't support.
	if (header.Typeflag != tar.TypeReg) && (header.Typeflag != tar.TypeDir) && (header.Typeflag != tar.TypeSymlink) {
		return errUnsupportedFileType
	}

	// Consistent creation.
	header.Uid = 0
	header.Gid = 0
	header.AccessTime = time.Unix(0, 0)
	header.ModTime = time.Unix(0, 0)
	header.ChangeTime = time.Unix(0, 0)

	// Always write the header.
	if err := ci.tw.WriteHeader(header); err != nil {
		return err
	}

	// If there is a body to be written, do so.
	if header.Typeflag == tar.TypeReg && header.Size > 0 {
		// Windows has a distinct "sequential read" opening mode.
		// We use a library that will switch to this mode for Windows.
		sourceFile, sourceErr := sequential.OpenFile(sourcePath.ToString(), os.O_RDONLY, 0777)
		if sourceErr != nil {
			return sourceErr
		}

		if _, err := io.Copy(ci.tw, sourceFile); err != nil {
			return err
		}

		return sourceFile.Close()
	}

	return nil
}
