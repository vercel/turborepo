package cacheitem

import (
	"archive/tar"
	"errors"
	"io"
	"os"
	"runtime"
	"strings"

	"github.com/DataDog/zstd"

	"github.com/moby/sys/sequential"
	"github.com/vercel/turborepo/cli/internal/turbopath"
)

// Open returns an existing CacheItem at the specified path.
func Open(path turbopath.AbsoluteSystemPath) (*CacheItem, error) {
	handle, err := sequential.OpenFile(path.ToString(), os.O_RDONLY, 0777)
	if err != nil {
		return nil, err
	}

	return &CacheItem{
		Path:       path,
		handle:     handle,
		compressed: strings.HasSuffix(path.ToString(), ".zst"),
	}, nil
}

// Restore extracts a cache to a specified disk location.
func (ci *CacheItem) Restore(anchor turbopath.AbsoluteSystemPath) ([]turbopath.AnchoredSystemPath, error) {
	var tr *tar.Reader
	var closeError error

	// We're reading a tar, possibly wrapped in zstd.
	if ci.compressed {
		zr := zstd.NewReader(ci.handle)

		// The `Close` function for compression effectively just returns the singular
		// error field on the decompressor instance. This is extremely unlikely to be
		// set without triggering one of the numerous other errors, but we should still
		// handle that possible edge case.
		defer func() { closeError = zr.Close() }()
		tr = tar.NewReader(zr)
	} else {
		tr = tar.NewReader(ci.handle)
	}

	// On first attempt to restore it's possible that a link target doesn't exist.
	// Save them and topsort them.
	var symlinks []*tar.Header

	restored := make([]turbopath.AnchoredSystemPath, 0)

	restorePointErr := anchor.MkdirAll(0755)
	if restorePointErr != nil {
		return nil, restorePointErr
	}

	// We're going to make the following two assumptions here for "fast" path restoration:
	// - All directories are enumerated in the `tar`.
	// - The contents of the tar are enumerated depth-first.
	//
	// This allows us to avoid:
	// - Attempts at recursive creation of directories.
	// - Repetitive `lstat` on restore of a file.
	//
	// Violating these assumptions won't cause things to break but we're only going to maintain
	// an `lstat` cache for the current tree. If you violate these assumptions and the current
	// cache does not apply for your path, it will clobber and re-start from the common
	// shared prefix.
	dirCache := &cachedDirTree{
		anchorAtDepth: []turbopath.AbsoluteSystemPath{anchor},
	}

	for {
		header, trErr := tr.Next()
		if trErr == io.EOF {
			// The end, time to restore any missing links.
			symlinksRestored, symlinksErr := topologicallyRestoreSymlinks(dirCache, anchor, symlinks, tr)
			restored = append(restored, symlinksRestored...)
			if symlinksErr != nil {
				return restored, symlinksErr
			}

			break
		}
		if trErr != nil {
			return restored, trErr
		}

		// The reader will not advance until tr.Next is called.
		// We can treat this as file metadata + body reader.

		// Attempt to place the file on disk.
		file, restoreErr := restoreEntry(dirCache, anchor, header, tr)
		if restoreErr != nil {
			if errors.Is(restoreErr, errMissingSymlinkTarget) {
				// Links get one shot to be valid, then they're accumulated, DAG'd, and restored on delay.
				symlinks = append(symlinks, header)
				continue
			}
			return restored, restoreErr
		}
		restored = append(restored, file)
	}

	return restored, closeError
}

// restoreRegular is the entry point for all things read from the tar.
func restoreEntry(dirCache *cachedDirTree, anchor turbopath.AbsoluteSystemPath, header *tar.Header, reader *tar.Reader) (turbopath.AnchoredSystemPath, error) {
	// We're permissive on creation, but restrictive on restoration.
	// There is no need to prevent the cache creation in any case.
	// And on restoration, if we fail, we simply run the task.
	switch header.Typeflag {
	case tar.TypeDir:
		return restoreDirectory(dirCache, anchor, header)
	case tar.TypeReg:
		return restoreRegular(dirCache, anchor, header, reader)
	case tar.TypeSymlink:
		return restoreSymlink(dirCache, anchor, header)
	default:
		return "", errUnsupportedFileType
	}
}

// canonicalizeName returns either an AnchoredSystemPath or an error.
func canonicalizeName(name string) (turbopath.AnchoredSystemPath, error) {
	// Assuming this was a `turbo`-created input, we currently have an AnchoredUnixPath.
	// Assuming this is malicious input we don't really care if we do the wrong thing.
	wellFormed, windowsSafe := checkName(name)

	// Determine if the future filename is a well-formed AnchoredUnixPath
	if !wellFormed {
		return "", errNameMalformed
	}

	// Determine if the AnchoredUnixPath is safe to be used on Windows
	if runtime.GOOS == "windows" && !windowsSafe {
		return "", errNameWindowsUnsafe
	}

	// Directories will have a trailing slash. Remove it.
	noTrailingSlash := strings.TrimSuffix(name, "/")

	// Okay, we're all set here.
	return turbopath.AnchoredUnixPathFromUpstream(noTrailingSlash).ToSystemPath(), nil
}

// checkName returns `wellFormed, windowsSafe` via inspection of separators and traversal
func checkName(name string) (bool, bool) {
	length := len(name)

	// Name is of length 0.
	if length == 0 {
		return false, false
	}

	wellFormed := true
	windowsSafe := true

	// Name is:
	// - "."
	// - ".."
	if wellFormed && (name == "." || name == "..") {
		wellFormed = false
	}

	// Name starts with:
	// - `/`
	// - `./`
	// - `../`
	if wellFormed && (strings.HasPrefix(name, "/") || strings.HasPrefix(name, "./") || strings.HasPrefix(name, "../")) {
		wellFormed = false
	}

	// Name ends in:
	// - `/.`
	// - `/..`
	if wellFormed && (strings.HasSuffix(name, "/.") || strings.HasSuffix(name, "/..")) {
		wellFormed = false
	}

	// Name contains:
	// - `//`
	// - `/./`
	// - `/../`
	if wellFormed && (strings.Contains(name, "//") || strings.Contains(name, "/./") || strings.Contains(name, "/../")) {
		wellFormed = false
	}

	// Name contains: `\`
	if strings.ContainsRune(name, '\\') {
		windowsSafe = false
	}

	return wellFormed, windowsSafe
}
