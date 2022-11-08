package cacheitem

import (
	"archive/tar"
	"io/fs"
	"os"
	"path/filepath"

	"github.com/pyr-sh/dag"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// restoreSymlink restores a symlink and errors if the target is missing.
func restoreSymlink(dirCache *cachedDirTree, anchor turbopath.AbsoluteSystemPath, header *tar.Header) (turbopath.AnchoredSystemPath, error) {
	processedName, canonicalizeNameErr := canonicalizeName(header.Name)
	if canonicalizeNameErr != nil {
		return "", canonicalizeNameErr
	}

	// Check to see if the target exists.
	processedLinkname := canonicalizeLinkname(anchor, processedName, header.Linkname)
	if _, err := os.Lstat(processedLinkname); err != nil {
		return "", errMissingSymlinkTarget
	}

	return actuallyRestoreSymlink(dirCache, anchor, processedName, header)
}

// restoreSymlinkMissingTarget restores a symlink and does not error if the target is missing.
func restoreSymlinkMissingTarget(dirCache *cachedDirTree, anchor turbopath.AbsoluteSystemPath, header *tar.Header) (turbopath.AnchoredSystemPath, error) {
	processedName, canonicalizeNameErr := canonicalizeName(header.Name)
	if canonicalizeNameErr != nil {
		return "", canonicalizeNameErr
	}

	return actuallyRestoreSymlink(dirCache, anchor, processedName, header)
}

func actuallyRestoreSymlink(dirCache *cachedDirTree, anchor turbopath.AbsoluteSystemPath, processedName turbopath.AnchoredSystemPath, header *tar.Header) (turbopath.AnchoredSystemPath, error) {
	// We need to traverse `processedName` from base to root split at
	// `os.Separator` to make sure we don't end up following a symlink
	// outside of the restore path.
	if err := safeMkdirFile(dirCache, anchor, processedName, header.Mode); err != nil {
		return "", err
	}

	// Specify where we restoring this symlink.
	symlinkFrom := processedName.RestoreAnchor(anchor)

	// Remove any existing object at that location.
	// If it errors we'll catch it on creation.
	_ = symlinkFrom.Remove()

	// Create the symlink.
	// Explicitly uses the _original_ header.Linkname as the target.
	// This does not support file names with `\` in them in a cross-platform manner.
	symlinkErr := symlinkFrom.Symlink(header.Linkname)
	if symlinkErr != nil {
		return "", symlinkErr
	}

	// Darwin allows you to change the permissions of a symlink.
	lchmodErr := symlinkFrom.Lchmod(fs.FileMode(header.Mode))
	if lchmodErr != nil {
		return "", lchmodErr
	}

	return processedName, nil
}

// topologicallyRestoreSymlinks ensures that targets of symlinks are created in advance
// of the things that link to them. It does this by topologically sorting all
// of the symlinks. This also enables us to ensure we do not create cycles.
func topologicallyRestoreSymlinks(dirCache *cachedDirTree, anchor turbopath.AbsoluteSystemPath, symlinks []*tar.Header, tr *tar.Reader) ([]turbopath.AnchoredSystemPath, error) {
	restored := make([]turbopath.AnchoredSystemPath, 0)
	lookup := make(map[string]*tar.Header)

	var g dag.AcyclicGraph
	for _, header := range symlinks {
		processedName, err := canonicalizeName(header.Name)
		processedSourcename := canonicalizeLinkname(anchor, processedName, processedName.ToString())
		processedLinkname := canonicalizeLinkname(anchor, processedName, header.Linkname)
		if err != nil {
			return nil, err
		}
		g.Add(processedSourcename)
		g.Add(processedLinkname)
		g.Connect(dag.BasicEdge(processedLinkname, processedSourcename))
		lookup[processedSourcename] = header
	}

	cycles := g.Cycles()
	if cycles != nil {
		return restored, errCycleDetected
	}

	roots := make(dag.Set)
	for _, v := range g.Vertices() {
		if g.UpEdges(v).Len() == 0 {
			roots.Add(v)
		}
	}

	walkFunc := func(vertex dag.Vertex, depth int) error {
		key, ok := vertex.(string)
		if !ok {
			return nil
		}
		header, exists := lookup[key]
		if !exists {
			return nil
		}

		file, restoreErr := restoreSymlinkMissingTarget(dirCache, anchor, header)
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

// canonicalizeLinkname determines (lexically) what the resolved path on the
// system will be when linkname is restored verbatim.
func canonicalizeLinkname(anchor turbopath.AbsoluteSystemPath, processedName turbopath.AnchoredSystemPath, linkname string) string {
	// We don't know _anything_ about linkname. It could be any of:
	//
	// - Absolute Unix Path
	// - Absolute Windows Path
	// - Relative Unix Path
	// - Relative Windows Path
	//
	// We also can't _truly_ distinguish if the path is Unix or Windows.
	// Take for example: `/Users/turbobot/weird-filenames/\foo\/lol`
	// It is a valid file on Unix, but if we do slash conversion it breaks.
	// Or `i\am\a\normal\unix\file\but\super\nested\on\windows`.
	//
	// We also can't safely assume that paths in link targets on one platform
	// should be treated as targets for that platform. The author may be
	// generating an artifact that should work on Windows on a Unix device.
	//
	// Given all of that, our best option is to restore link targets _verbatim_.
	// No modification, no slash conversion.
	//
	// In order to DAG sort them, however, we do need to canonicalize them.
	// We canonicalize them as if we're restoring them verbatim.
	//
	// 0. We've extracted a version of `Clean` from stdlib which does nothing but
	// separator and traversal collapsing.
	cleanedLinkname := Clean(linkname)

	// 1. Check to see if the link target is absolute _on the current platform_.
	// If it is an absolute path it's canonical by rule.
	if filepath.IsAbs(cleanedLinkname) {
		return cleanedLinkname
	}

	// Remaining options:
	// - Absolute (other platform) Path
	// - Relative Unix Path
	// - Relative Windows Path
	//
	// At this point we simply assume that it's a relative path—no matter
	// which separators appear in it and where they appear,  We can't do
	// anything else because the OS will also treat it like that when it is
	// a link target.
	//
	// We manually join these to avoid calls to stdlib's `Clean`.
	source := processedName.RestoreAnchor(anchor)
	canonicalized := source.Dir().ToString() + string(os.PathSeparator) + cleanedLinkname
	return Clean(canonicalized)
}
