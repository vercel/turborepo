package cacheitem

import (
	"archive/tar"
	"io"
	"os"

	"github.com/vercel/turborepo/cli/internal/turbopath"
)

// restoreRegular restores a file.
func restoreRegular(dirCache *cachedDirTree, anchor turbopath.AbsoluteSystemPath, header *tar.Header, reader *tar.Reader) (turbopath.AnchoredSystemPath, error) {
	// Assuming this was a `turbo`-created input, we currently have an AnchoredUnixPath.
	// Assuming this is malicious input we don't really care if we do the wrong thing.
	processedName, err := canonicalizeName(header.Name)
	if err != nil {
		return "", err
	}

	// We need to traverse `processedName` from base to root split at
	// `os.Separator` to make sure we don't end up following a symlink
	// outside of the restore path.
	if err := safeMkdirFile(dirCache, anchor, processedName, header.Mode); err != nil {
		return "", err
	}

	// Create the file.
	if f, err := processedName.RestoreAnchor(anchor).OpenFile(os.O_WRONLY|os.O_TRUNC|os.O_CREATE, os.FileMode(header.Mode)); err != nil {
		return "", err
	} else if _, err := io.Copy(f, reader); err != nil {
		return "", err
	} else if err := f.Close(); err != nil {
		return "", err
	}
	return processedName, nil
}

// safeMkdirAll creates all directories, assuming that the leaf node is a file.
func safeMkdirFile(dirCache *cachedDirTree, anchor turbopath.AbsoluteSystemPath, processedName turbopath.AnchoredSystemPath, mode int64) error {
	isRootFile := processedName.Dir() == "."
	if !isRootFile {
		return safeMkdirAll(dirCache, anchor, processedName.Dir(), 0755)
	}

	return nil
}
