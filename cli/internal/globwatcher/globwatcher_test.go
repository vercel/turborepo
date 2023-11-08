package globwatcher

import (
	"testing"

	"github.com/hashicorp/go-hclog"
	"github.com/vercel/turbo/cli/internal/filewatcher"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/fs/hash"
	"github.com/vercel/turbo/cli/internal/turbopath"
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

	globs := hash.TaskOutputs{
		Inclusions: []string{
			"my-pkg/dist/**",
			"my-pkg/.next/**",
		},
		Exclusions: []string{"my-pkg/.next/cache/**"},
	}

	hash := "the-hash"
	err := globWatcher.WatchGlobs(hash, globs)
	assert.NilError(t, err, "WatchGlobs")

	changed, err := globWatcher.GetChangedGlobs(hash, globs.Inclusions)
	assert.NilError(t, err, "GetChangedGlobs")
	assert.Equal(t, 0, len(changed), "Expected no changed paths")

	// Make an irrelevant change
	globWatcher.OnFileWatchEvent(filewatcher.Event{
		EventType: filewatcher.FileAdded,
		Path:      repoRoot.UntypedJoin("my-pkg", "irrelevant"),
	})

	changed, err = globWatcher.GetChangedGlobs(hash, globs.Inclusions)
	assert.NilError(t, err, "GetChangedGlobs")
	assert.Equal(t, 0, len(changed), "Expected no changed paths")

	// Make an excluded change
	globWatcher.OnFileWatchEvent(filewatcher.Event{
		EventType: filewatcher.FileAdded,
		Path:      repoRoot.Join("my-pkg", ".next", "cache", "foo"),
	})

	changed, err = globWatcher.GetChangedGlobs(hash, globs.Inclusions)
	assert.NilError(t, err, "GetChangedGlobs")
	assert.Equal(t, 0, len(changed), "Expected no changed paths")

	// Make a relevant change
	globWatcher.OnFileWatchEvent(filewatcher.Event{
		EventType: filewatcher.FileAdded,
		Path:      repoRoot.UntypedJoin("my-pkg", "dist", "foo"),
	})

	changed, err = globWatcher.GetChangedGlobs(hash, globs.Inclusions)
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
	changed, err = globWatcher.GetChangedGlobs(hash, globs.Inclusions)
	assert.NilError(t, err, "GetChangedGlobs")
	assert.DeepEqual(t, globs.Inclusions, changed)
}

func TestTrackMultipleHashes(t *testing.T) {
	logger := hclog.Default()

	repoRootRaw := t.TempDir()
	repoRoot := fs.AbsoluteSystemPathFromUpstream(repoRootRaw)

	setup(t, repoRoot)

	globWatcher := New(logger, repoRoot, _noopCookieWaiter)

	globs := hash.TaskOutputs{
		Inclusions: []string{
			"my-pkg/dist/**",
			"my-pkg/.next/**",
		},
	}

	hashToWatch := "the-hash"
	err := globWatcher.WatchGlobs(hashToWatch, globs)
	assert.NilError(t, err, "WatchGlobs")

	secondGlobs := hash.TaskOutputs{
		Inclusions: []string{
			"my-pkg/.next/**",
		},
		Exclusions: []string{"my-pkg/.next/cache/**"},
	}

	secondHash := "the-second-hash"
	err = globWatcher.WatchGlobs(secondHash, secondGlobs)
	assert.NilError(t, err, "WatchGlobs")

	changed, err := globWatcher.GetChangedGlobs(hashToWatch, globs.Inclusions)
	assert.NilError(t, err, "GetChangedGlobs")
	assert.Equal(t, 0, len(changed), "Expected no changed paths")

	changed, err = globWatcher.GetChangedGlobs(secondHash, secondGlobs.Inclusions)
	assert.NilError(t, err, "GetChangedGlobs")
	assert.Equal(t, 0, len(changed), "Expected no changed paths")

	// Make a change that is excluded in one of the hashes but not in the other
	globWatcher.OnFileWatchEvent(filewatcher.Event{
		EventType: filewatcher.FileAdded,
		Path:      repoRoot.UntypedJoin("my-pkg", ".next", "cache", "foo"),
	})

	changed, err = globWatcher.GetChangedGlobs(hashToWatch, globs.Inclusions)
	assert.NilError(t, err, "GetChangedGlobs")
	assert.Equal(t, 1, len(changed), "Expected one changed path remaining")

	changed, err = globWatcher.GetChangedGlobs(secondHash, secondGlobs.Inclusions)
	assert.NilError(t, err, "GetChangedGlobs")
	assert.Equal(t, 0, len(changed), "Expected no changed paths")

	assert.Equal(t, 1, len(globWatcher.globStatus["my-pkg/.next/**"]), "Expected to be still watching `my-pkg/.next/**`")

	// Make a change for secondHash
	globWatcher.OnFileWatchEvent(filewatcher.Event{
		EventType: filewatcher.FileAdded,
		Path:      repoRoot.UntypedJoin("my-pkg", ".next", "bar"),
	})

	assert.Equal(t, 0, len(globWatcher.globStatus["my-pkg/.next/**"]), "Expected to be no longer watching `my-pkg/.next/**`")
}

func TestWatchSingleFile(t *testing.T) {
	logger := hclog.Default()

	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())

	setup(t, repoRoot)

	//watcher := newTestWatcher()
	globWatcher := New(logger, repoRoot, _noopCookieWaiter)
	globs := hash.TaskOutputs{
		Inclusions: []string{"my-pkg/.next/next-file"},
		Exclusions: []string{},
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
