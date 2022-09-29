package cacheitem

import (
	"archive/tar"
	"os"
	"path/filepath"
	"strings"

	"github.com/vercel/turborepo/cli/internal/turbopath"
)

// restoreDirectory restores a directory.
func restoreDirectory(anchor turbopath.AbsoluteSystemPath, header *tar.Header, reader *tar.Reader) (turbopath.AnchoredSystemPath, error) {
	processedName, err := canonicalizeName(header.Name)
	if err != nil {
		return "", err
	}

	// We need to traverse `processedName` from base to root split at
	// `os.Separator` to make sure we don't end up following a symlink
	// outside of the restore path.

	// Create the directory.
	if err := safeMkdirAll(anchor, processedName, header.Mode); err != nil {
		return "", err
	}

	return processedName, nil
}

// safeMkdirAll creates all directories, assuming that the leaf node is a directory.
// FIXME: Recheck the symlink cache before creating a directory.
func safeMkdirAll(anchor turbopath.AbsoluteSystemPath, processedName turbopath.AnchoredSystemPath, mode int64) error {
	// Iterate through path segments by os.Separator, appending them onto the anchor.
	// Check to see if that path segment is a symlink with a target outside of anchor.
	pathSegments := strings.Split(processedName.ToString(), string(os.PathSeparator))

	calculatedAnchor := anchor
	var checkPathErr error
	for _, segment := range pathSegments {
		calculatedAnchor, checkPathErr = checkPath(anchor, calculatedAnchor, turbopath.RelativeSystemPath(segment))
		// We hit an existing directory or absolute path that was invalid.
		if checkPathErr != nil {
			return checkPathErr
		}

		// Otherwise we continue and check the next segment.
	}

	// If we have made it here we know that it is safe to call os.MkdirAll
	// on the Join of anchor and processedName.
	//
	// This could _still_ error, but we don't care.
	return processedName.RestoreAnchor(anchor).MkdirAll(os.FileMode(mode))
}

// checkPath ensures that the resolved path (if restoring symlinks).
// It makes sure to never traverse outside of the anchor.
func checkPath(originalAnchor turbopath.AbsoluteSystemPath, accumulatedAnchor turbopath.AbsoluteSystemPath, segment turbopath.RelativeSystemPath) (turbopath.AbsoluteSystemPath, error) {
	// Check if the segment itself is sneakily an absolute path...
	// (looking at you, Windows. CON, AUX...)
	if filepath.IsAbs(segment.ToString()) {
		return "", errTraversal
	}

	// Find out if this portion of the path is a symlink.
	combinedPath := accumulatedAnchor.Join(segment)
	fileInfo, err := combinedPath.Lstat()

	// Getting an error here means we failed to stat the path.
	// Assume that means we're safe and continue.
	if err != nil {
		return combinedPath, nil
	}

	// Find out if we have a symlink.
	isSymlink := fileInfo.Mode()&os.ModeSymlink != 0

	// If we don't have a symlink it's safe.
	if !isSymlink {
		return combinedPath, nil
	}

	// Check to see if the symlink targets outside of the originalAnchor.
	// We don't do eval symlinks because we could find ourself in a totally
	// different place.

	// 1. Get the target.
	linkTarget, readLinkErr := combinedPath.Readlink()
	if readLinkErr != nil {
		return "", readLinkErr
	}

	// 2. See if the target is absolute.
	if filepath.IsAbs(linkTarget) {
		if strings.HasPrefix(linkTarget, originalAnchor.ToString()) {
			return turbopath.AbsoluteSystemPath(linkTarget), nil
		}
		return "", errTraversal
	}

	// 3. Target is relative (or absolute Windows on a Unix device)
	computedTarget := filepath.Join(accumulatedAnchor.ToString(), linkTarget)
	if strings.HasPrefix(computedTarget, originalAnchor.ToString()) {
		return turbopath.AbsoluteSystemPath(computedTarget), nil
	}

	return "", errTraversal
}
