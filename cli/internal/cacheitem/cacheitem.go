package cacheitem

import (
	"archive/tar"
	"compress/gzip"
	"crypto/sha512"
	"errors"
	"hash"
	"io"
	"io/fs"
	"log"
	"os"
	"sync"

	"github.com/moby/sys/sequential"
	"github.com/pyr-sh/dag"
	"github.com/vercel/turborepo/cli/internal/turbopath"
)

var (
	errNonexistentLinkTarget = errors.New("the link target does not exist")
	errCycleDetected         = errors.New("symlinks in the cache are cyclic")
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

// New CacheItem

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

// Existing CacheItem

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

	// On first attempt to restore it's possible that a link target doesn't exist.
	// Save them and come back to them.
	missingLinks := make(map[string]*tar.Header)

	restored := make([]turbopath.AnchoredSystemPath, 0)
	for {
		header, trErr := tr.Next()
		if trErr == io.EOF {
			// The end, time to restore the missing links.
			missingLinksRestored, missingLinksErr := restoreMissingLinks(missingLinks, tr)
			restored = append(restored, missingLinksRestored...)
			if missingLinksErr != nil {
				return restored, missingLinksErr
			}

			break
		}
		if trErr != nil {
			return restored, trErr
		}

		// The reader will not advance until tr.Next is called.
		// We can treat this as file metadata + body reader.

		// Make sure that we pass our safety checks.
		validateErr := validateEntry(header, tr)
		if validateErr != nil {
			return restored, validateErr
		}

		// Actually attempt to place the file on disk.
		file, restoreErr := restoreEntry(header, tr)
		if restoreErr != nil {
			if errors.Is(restoreErr, errNonexistentLinkTarget) {
				// Links get one shot to be valid, then they're DAG'd and delayed.
				missingLinks[header.Name] = header
				continue
			}
			return restored, restoreErr
		}
		restored = append(restored, file)
	}

	return restored, nil
}

func validateEntry(header *tar.Header, reader *tar.Reader) error {
	return nil
}

func restoreMissingLinks(missingLinks map[string]*tar.Header, tr *tar.Reader) ([]turbopath.AnchoredSystemPath, error) {
	restored := make([]turbopath.AnchoredSystemPath, 0)

	var g dag.AcyclicGraph
	for _, header := range missingLinks {
		g.Add(header.Name)
	}
	for key, header := range missingLinks {
		g.Connect(dag.BasicEdge(key, header.Name))
	}

	cycles := g.Cycles()
	if cycles != nil {
		return restored, errCycleDetected
	}

	var roots dag.Set
	for _, v := range g.Vertices() {
		if g.UpEdges(v).Len() == 0 {
			roots.Add(v)
		}
	}

	var walkFunc dag.DepthWalkFunc
	walkFunc = func(vertex dag.Vertex, depth int) error {
		header := vertex.(*tar.Header)
		file, restoreErr := restoreEntry(header, tr)
		if restoreErr != nil {
			return restoreErr
		}

		restored = append(restored, file)
		return nil
	}
	walkError := g.DepthFirstWalk(roots, walkFunc)
	if walkError != nil {
		return restored, walkError
	}

	return restored, nil
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
