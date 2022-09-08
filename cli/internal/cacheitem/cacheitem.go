package cacheitem

import (
	"archive/tar"
	"compress/gzip"
	"crypto/sha512"
	"hash"
	"io"
	"log"
	"os"
	"path/filepath"
	"sync"

	"github.com/vercel/turborepo/cli/internal/turbopath"
)

// CacheItem is a `tar` utility with a little bit extra.
type CacheItem struct {
	// Path is the location on disk for the CacheItem.
	Path turbopath.AbsoluteSystemPath
	// Anchor is the position on disk at which the CacheItem will be restored.
	Anchor turbopath.AbsoluteSystemPath

	// For creation.
	once   sync.Once
	sha    hash.Hash
	tw     *tar.Writer
	gzw    *gzip.Writer
	handle *os.File
}

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
func (ci *CacheItem) AddMetadata(path turbopath.AnchoredSystemPath) {
	ci.init()
	ci.addFile("metadata", path)
}

// AddFile adds a user-cached item to the tar.
func (ci *CacheItem) AddFile(path turbopath.AnchoredSystemPath) {
	ci.init()
	ci.addFile("cache", path)
}

func (ci *CacheItem) addFile(prefix string, path turbopath.AnchoredSystemPath) {
	// Cache structure forces files into a separate directory.
	var files = []struct {
		Name, Body string
	}{
		{"readme.txt", "This archive contains some text files."},
		{"gopher.txt", "Gopher names:\nGeorge\nGeoffrey\nGonzo"},
		{"todo.txt", "Get animal handling license."},
	}
	for _, file := range files {
		hdr := &tar.Header{
			Name: filepath.Join(prefix, file.Name),
			Mode: 0600,
			Size: int64(len(file.Body)),
		}
		if err := ci.tw.WriteHeader(hdr); err != nil {
			log.Fatal(err)
		}
		if _, err := ci.tw.Write([]byte(file.Body)); err != nil {
			log.Fatal(err)
		}
	}
}

// EXISTING

// Open returns an existing CacheItem at the specified path.
func Open(path turbopath.AbsoluteSystemPath) (*CacheItem, error) {
	handle, err := os.Open(path.ToString())
	if err != nil {
		return nil, err
	}

	return &CacheItem{
		Path:   path,
		handle: handle,
	}, nil
}

// Restore extracts a cache to a specified disk location.
func (ci *CacheItem) Restore(anchor turbopath.AbsoluteSystemPath) ([]turbopath.AnchoredSystemPath, error) {
	// tar wrapped in gzip, we need to stream out of gzip first.
	gzr, err := gzip.NewReader(ci.handle)
	if err != nil {
		return nil, err
	}
	defer func() { _ = gzr.Close() }()
	tr := tar.NewReader(gzr)

	restored := make([]turbopath.AnchoredSystemPath, 0)
	for {
		header, trErr := tr.Next()
		if trErr == io.EOF {
			break // End of archive
		}
		if trErr != nil {
			return restored, trErr
		}

		// The reader will not advance until tr.Next is called.
		// We can treat this as file metadata + body reader.
		validateErr := validateEntry(header, tr)
		if validateErr != nil {
			return restored, validateErr
		}
		file, restoreErr := restoreEntry(header, tr)
		if restoreErr != nil {
			return restored, restoreErr
		}
		restored = append(restored, file)
	}

	return restored, nil
}

func validateEntry(header *tar.Header, reader *tar.Reader) error {
	return nil
}

func restoreEntry(header *tar.Header, reader *tar.Reader) (turbopath.AnchoredSystemPath, error) {
	switch header.Typeflag {
	case tar.TypeDir:
	case tar.TypeReg:
	case tar.TypeSymlink:
	}
	return "", nil
}

// SHARED

// Close any open pipes
func (ci *CacheItem) Close() error {
	// Close from the beginning of the pipe to the end.
	closers := []io.Closer{ci.tw, ci.gzw, ci.handle}

	for _, closer := range closers {
		// Skip the things which may not exist in this particular instance.
		if closer == nil {
			continue
		}
		if err := closer.Close(); err != nil {
			return err
		}
	}

	return nil
}

// GetSha returns the SHA-512 hash for the CacheItem.
func (ci *CacheItem) GetSha() []byte {
	if ci.sha != nil {
		return ci.sha.Sum(nil)
	}

	sha := sha512.New()
	if _, err := io.Copy(sha, ci.handle); err != nil {
		log.Fatal(err)
	}

	// Don't mutate the sha until it will return the correct value.
	ci.sha = sha

	return ci.sha.Sum(nil)
}
