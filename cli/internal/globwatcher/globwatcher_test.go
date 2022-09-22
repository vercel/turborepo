package globwatcher

import (
	"testing"

	"github.com/hashicorp/go-hclog"
	"github.com/vercel/turborepo/cli/internal/filewatcher"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
)

func setup(t *testing.T, repoRoot turbopath.AbsoluteSystemPath) {
	// Directory layout:
	// <repoRoot>/
	//   my-pkg/
	//     irrelevant
	//     dist/
	//       dist-file
	//       distChild/
	//         child-file
	//     .next/
	//       next-file
	distPath := repoRoot.UntypedJoin("my-pkg", "dist")
	childFilePath := distPath.UntypedJoin("distChild", "child-file")
	err := childFilePath.EnsureDir()
	assert.NilError(t, err, "EnsureDir")
	f, err := childFilePath.Create()
	assert.NilError(t, err, "Create")
	err = f.Close()
	assert.NilError(t, err, "Close")
	distFilePath := repoRoot.UntypedJoin("my-pkg", "dist", "dist-file")
	f, err = distFilePath.Create()
	assert.NilError(t, err, "Create")
	err = f.Close()
	assert.NilError(t, err, "Close")
	nextFilePath := repoRoot.UntypedJoin("my-pkg", ".next", "next-file")
	err = nextFilePath.EnsureDir()
	assert.NilError(t, err, "EnsureDir")
	f, err = nextFilePath.Create()
	assert.NilError(t, err, "Create")
	err = f.Close()
	assert.NilError(t, err, "Close")
	irrelevantPath := repoRoot.UntypedJoin("my-pkg", "irrelevant")
	f, err = irrelevantPath.Create()
	assert.NilError(t, err, "Create")
	err = f.Close()
	assert.NilError(t, err, "Close")
}

type noopCookieWaiter struct{}

func (*noopCookieWaiter) WaitForCookie() error {
	return nil
}

var _noopCookieWaiter = &noopCookieWaiter{}

func TestTrackOutputs(t *testing.T) {
	logger := hclog.Default()

	repoRootRaw := t.TempDir()
	repoRoot := fs.AbsoluteSystemPathFromUpstream(repoRootRaw)

	setup(t, repoRoot)

	globWatcher := New(logger, repoRoot, _noopCookieWaiter)

	globs := []string{
		"my-pkg/dist/**",
		"my-pkg/.next/**",
	}
	hash := "the-hash"
	err := globWatcher.WatchGlobs(hash, globs)
	assert.NilError(t, err, "WatchGlobs")

	changed, err := globWatcher.GetChangedGlobs(hash, globs)
	assert.NilError(t, err, "GetChangedGlobs")
	assert.Equal(t, 0, len(changed), "Expected no changed paths")

	// Make an irrelevant change
	globWatcher.OnFileWatchEvent(filewatcher.Event{
		EventType: filewatcher.FileAdded,
		Path:      repoRoot.UntypedJoin("my-pkg", "irrelevant"),
	})

	changed, err = globWatcher.GetChangedGlobs(hash, globs)
	assert.NilError(t, err, "GetChangedGlobs")
	assert.Equal(t, 0, len(changed), "Expected no changed paths")

	// Make a relevant change
	globWatcher.OnFileWatchEvent(filewatcher.Event{
		EventType: filewatcher.FileAdded,
		Path:      repoRoot.UntypedJoin("my-pkg", "dist", "foo"),
	})

	changed, err = globWatcher.GetChangedGlobs(hash, globs)
	assert.NilError(t, err, "GetChangedGlobs")
	assert.Equal(t, 1, len(changed), "Expected one changed path remaining")
	expected := "my-pkg/dist/**"
	assert.Equal(t, expected, changed[0], "Expected dist glob to have changed")

	// Change a file matching the other glob
	globWatcher.OnFileWatchEvent(filewatcher.Event{
		EventType: filewatcher.FileAdded,
		Path:      repoRoot.UntypedJoin("my-pkg", ".next", "foo"),
	})
	// We should no longer be watching anything, since both globs have
	// registered changes
	if len(globWatcher.hashGlobs) != 0 {
		t.Errorf("expected to not track any hashes, found %v", globWatcher.hashGlobs)
	}

	// Both globs have changed, we should have stopped tracking
	// this hash
	changed, err = globWatcher.GetChangedGlobs(hash, globs)
	assert.NilError(t, err, "GetChangedGlobs")
	assert.DeepEqual(t, globs, changed)
}

func TestWatchSingleFile(t *testing.T) {
	logger := hclog.Default()

	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())

	setup(t, repoRoot)

	//watcher := newTestWatcher()
	globWatcher := New(logger, repoRoot, _noopCookieWaiter)
	globs := []string{
		"my-pkg/.next/next-file",
	}
	hash := "the-hash"
	err := globWatcher.WatchGlobs(hash, globs)
	assert.NilError(t, err, "WatchGlobs")

	assert.Equal(t, 1, len(globWatcher.hashGlobs))

	// A change to an irrelevant file
	globWatcher.OnFileWatchEvent(filewatcher.Event{
		EventType: filewatcher.FileAdded,
		Path:      repoRoot.UntypedJoin("my-pkg", ".next", "foo"),
	})
	assert.Equal(t, 1, len(globWatcher.hashGlobs))

	// Change the watched file
	globWatcher.OnFileWatchEvent(filewatcher.Event{
		EventType: filewatcher.FileAdded,
		Path:      repoRoot.UntypedJoin("my-pkg", ".next", "next-file"),
	})
	assert.Equal(t, 0, len(globWatcher.hashGlobs))
}
