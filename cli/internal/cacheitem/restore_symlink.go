package cacheitem

import (
	"archive/tar"
	"os"
	"path/filepath"

	"github.com/pyr-sh/dag"
	"github.com/vercel/turborepo/cli/internal/turbopath"
)

// restoreSymlink restores a symlink
func restoreSymlink(anchor turbopath.AbsoluteSystemPath, header *tar.Header, reader *tar.Reader) (turbopath.AnchoredSystemPath, error) {
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
	processedName, err := canonicalizeName(header.Name)
	if err != nil {
		return "", err
	}

	// Create the symlink.
	symlinkErr := os.Symlink(header.Linkname, processedName.RestoreAnchor(anchor).ToString())
	if symlinkErr != nil {
		return "", symlinkErr
	}

	return processedName, nil
}

// topologicalSortLinks ensures that targets of symlinks are created in advance
// of the things that link to them. It does this by topologically sorting all
// of the symlinks. This also enables us to ensure we do not create cycles.
func topologicalSortLinks(anchor turbopath.AbsoluteSystemPath, symlinks map[string]*tar.Header, tr *tar.Reader) ([]turbopath.AnchoredSystemPath, error) {
	restored := make([]turbopath.AnchoredSystemPath, 0)

	// FIXME: use canonical link names for the graph.

	var g dag.AcyclicGraph
	for _, header := range symlinks {
		g.Add(header.Name)
	}
	for key, header := range symlinks {
		g.Connect(dag.BasicEdge(key, header.Linkname))
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
		header, exists := symlinks[vertex.(string)]
		if !exists {
			return nil
		}

		file, restoreErr := restoreSymlink(anchor, header, tr)
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
