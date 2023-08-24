package filewatcher

import (
	"fmt"
	"runtime"
	"sync"
	"testing"
	"time"

	"github.com/hashicorp/go-hclog"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
)

type testClient struct {
	mu           sync.Mutex
	createEvents []Event
	notify       chan Event
}

func (c *testClient) OnFileWatchEvent(ev Event) {
	if ev.EventType == FileAdded {
		c.mu.Lock()
		defer c.mu.Unlock()
		c.createEvents = append(c.createEvents, ev)
	}
	if ev.EventType != FileModified {
		c.notify <- ev
	}
}

func (c *testClient) OnFileWatchError(err error) {}

func (c *testClient) OnFileWatchClosed() {}

func expectFilesystemEvent(t *testing.T, ch <-chan Event, expected Event) {
	// mark this method as a helper
	t.Helper()
	timeout := time.After(10 * time.Second)
	for {
		select {
		case ev := <-ch:
			t.Logf("got event %v", ev)
			if ev.Path == expected.Path && ev.EventType == expected.EventType {
				return
			}
		case <-timeout:
			t.Fatalf("Timed out waiting for filesystem event at %v %v", expected.EventType, expected.Path)
			return
		}
	}
}

func expectNoFilesystemEvent(t *testing.T, ch <-chan Event) {
	// mark this method as a helper
	t.Helper()
	select {
	case ev, ok := <-ch:
		if ok {
			t.Errorf("got unexpected filesystem event %v", ev)
		} else {
			t.Error("filewatching closed unexpectedly")
		}
	case <-time.After(500 * time.Millisecond):
		return
	}
}

// Hack to avoid duplicate filenames. Count the number of test files we create.
// Not thread-safe
var testFileCount = 0

func expectWatching(t *testing.T, c *testClient, dirs []turbopath.AbsoluteSystemPath) {
	t.Helper()
	thisFileCount := testFileCount
	testFileCount++
	filename := fmt.Sprintf("test-%v", thisFileCount)
	for _, dir := range dirs {
		file := dir.UntypedJoin(filename)
		err := file.WriteFile([]byte("hello"), 0755)
		assert.NilError(t, err, "WriteFile")
		expectFilesystemEvent(t, c.notify, Event{
			Path:      file,
			EventType: FileAdded,
		})
	}
}

func TestFileWatching(t *testing.T) {
	logger := hclog.Default()
	logger.SetLevel(hclog.Debug)
	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())
	err := repoRoot.UntypedJoin(".git").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	err = repoRoot.UntypedJoin("node_modules", "some-dep").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	err = repoRoot.UntypedJoin("parent", "child").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	err = repoRoot.UntypedJoin("parent", "sibling").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")

	// Directory layout:
	// <repoRoot>/
	//	 .git/
	//   node_modules/
	//     some-dep/
	//   parent/
	//     child/
	//     sibling/

	watcher, err := GetPlatformSpecificBackend(logger)
	assert.NilError(t, err, "GetPlatformSpecificBackend")
	fw := New(logger, repoRoot, watcher)
	err = fw.Start()
	assert.NilError(t, err, "fw.Start")

	// Add a client
	ch := make(chan Event, 1)
	c := &testClient{
		notify: ch,
	}
	fw.AddClient(c)
	expectedWatching := []turbopath.AbsoluteSystemPath{
		repoRoot,
		repoRoot.UntypedJoin("parent"),
		repoRoot.UntypedJoin("parent", "child"),
		repoRoot.UntypedJoin("parent", "sibling"),
	}
	expectWatching(t, c, expectedWatching)

	fooPath := repoRoot.UntypedJoin("parent", "child", "foo")
	err = fooPath.WriteFile([]byte("hello"), 0644)
	assert.NilError(t, err, "WriteFile")
	expectFilesystemEvent(t, ch, Event{
		EventType: FileAdded,
		Path:      fooPath,
	})

	deepPath := repoRoot.UntypedJoin("parent", "sibling", "deep", "path")
	err = deepPath.MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	// We'll catch an event for "deep", but not "deep/path" since
	// we don't have a recursive watch
	expectFilesystemEvent(t, ch, Event{
		Path:      repoRoot.UntypedJoin("parent", "sibling", "deep"),
		EventType: FileAdded,
	})
	expectFilesystemEvent(t, ch, Event{
		Path:      repoRoot.UntypedJoin("parent", "sibling", "deep", "path"),
		EventType: FileAdded,
	})
	expectedWatching = append(expectedWatching, deepPath, repoRoot.UntypedJoin("parent", "sibling", "deep"))
	expectWatching(t, c, expectedWatching)

	gitFilePath := repoRoot.UntypedJoin(".git", "git-file")
	err = gitFilePath.WriteFile([]byte("nope"), 0644)
	assert.NilError(t, err, "WriteFile")
	expectNoFilesystemEvent(t, ch)
}

// TestFileWatchingParentDeletion tests that when a repo subfolder is deleted,
// recursive watching will still work for new folders
//
// ✅ macOS
// ✅ Linux
// ✅ Windows
func TestFileWatchingSubfolderDeletion(t *testing.T) {
	logger := hclog.Default()
	logger.SetLevel(hclog.Debug)
	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())
	err := repoRoot.UntypedJoin(".git").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	err = repoRoot.UntypedJoin("node_modules", "some-dep").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	err = repoRoot.UntypedJoin("parent", "child").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")

	// Directory layout:
	// <repoRoot>/
	//	 .git/
	//   node_modules/
	//     some-dep/
	//   parent/
	//     child/

	watcher, err := GetPlatformSpecificBackend(logger)
	assert.NilError(t, err, "GetPlatformSpecificBackend")
	fw := New(logger, repoRoot, watcher)
	err = fw.Start()
	assert.NilError(t, err, "fw.Start")

	// Add a client
	ch := make(chan Event, 1)
	c := &testClient{
		notify: ch,
	}
	fw.AddClient(c)
	expectedWatching := []turbopath.AbsoluteSystemPath{
		repoRoot,
		repoRoot.UntypedJoin("parent"),
		repoRoot.UntypedJoin("parent", "child"),
	}
	expectWatching(t, c, expectedWatching)

	// Delete parent folder during file watching
	err = repoRoot.UntypedJoin("parent").RemoveAll()
	assert.NilError(t, err, "RemoveAll")

	// Ensure we don't get any event when creating file in deleted directory
	folder := repoRoot.UntypedJoin("parent", "child")
	err = folder.MkdirAllMode(0755)
	assert.NilError(t, err, "MkdirAll")

	expectFilesystemEvent(t, ch, Event{
		EventType: FileAdded,
		Path:      repoRoot.UntypedJoin("parent"),
	})

	expectFilesystemEvent(t, ch, Event{
		EventType: FileAdded,
		Path:      folder,
	})

	fooPath := folder.UntypedJoin("foo")
	err = fooPath.WriteFile([]byte("hello"), 0644)
	assert.NilError(t, err, "WriteFile")

	expectFilesystemEvent(t, ch, Event{
		EventType: FileAdded,
		Path:      folder.UntypedJoin("foo"),
	})
	// We cannot guarantee no more events, windows sends multiple delete events
}

// TestFileWatchingRootDeletion tests that when the root is deleted,
// we get a deleted event at the root.
//
// ✅ macOS
// ✅ Linux
// ❌ Windows - we do not get an event when the root is recreated L287
func TestFileWatchingRootDeletion(t *testing.T) {
	logger := hclog.Default()
	logger.SetLevel(hclog.Debug)
	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())
	err := repoRoot.UntypedJoin(".git").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	err = repoRoot.UntypedJoin("node_modules", "some-dep").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	err = repoRoot.UntypedJoin("parent", "child").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")

	// Directory layout:
	// <repoRoot>/
	//	 .git/
	//   node_modules/
	//     some-dep/
	//   parent/
	//     child/

	watcher, err := GetPlatformSpecificBackend(logger)
	assert.NilError(t, err, "GetPlatformSpecificBackend")
	fw := New(logger, repoRoot, watcher)
	err = fw.Start()
	assert.NilError(t, err, "fw.Start")

	// Add a client
	ch := make(chan Event, 1)
	c := &testClient{
		notify: ch,
	}
	fw.AddClient(c)
	expectedWatching := []turbopath.AbsoluteSystemPath{
		repoRoot,
		repoRoot.UntypedJoin("parent"),
		repoRoot.UntypedJoin("parent", "child"),
	}
	expectWatching(t, c, expectedWatching)

	// Delete parent folder during file watching
	err = repoRoot.RemoveAll()
	assert.NilError(t, err, "RemoveAll")

	expectFilesystemEvent(t, ch, Event{
		EventType: FileDeleted,
		Path:      repoRoot,
	})
}

// TestFileWatchingSubfolderRename tests that when a repo subfolder is renamed,
// file watching will continue, and a rename event will be sent.
//
// ✅ macOS
// ✅ Linux
// ❌ Windows - you cannot rename a watched folder (see https://github.com/fsnotify/fsnotify/issues/356)
func TestFileWatchingSubfolderRename(t *testing.T) {
	if runtime.GOOS == "windows" {
		return
	}
	logger := hclog.Default()
	logger.SetLevel(hclog.Debug)
	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())
	err := repoRoot.UntypedJoin(".git").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	err = repoRoot.UntypedJoin("node_modules", "some-dep").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	err = repoRoot.UntypedJoin("parent", "child").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")

	// Directory layout:
	// <repoRoot>/
	//	 .git/
	//   node_modules/
	//     some-dep/
	//   parent/
	//     child/

	watcher, err := GetPlatformSpecificBackend(logger)
	assert.NilError(t, err, "GetPlatformSpecificBackend")
	fw := New(logger, repoRoot, watcher)
	err = fw.Start()
	assert.NilError(t, err, "fw.Start")

	// Add a client
	ch := make(chan Event, 1)
	c := &testClient{
		notify: ch,
	}
	fw.AddClient(c)
	expectedWatching := []turbopath.AbsoluteSystemPath{
		repoRoot,
		repoRoot.UntypedJoin("parent"),
		repoRoot.UntypedJoin("parent", "child"),
	}
	expectWatching(t, c, expectedWatching)

	// Rename parent folder during file watching
	err = repoRoot.UntypedJoin("parent").Rename(repoRoot.UntypedJoin("new_parent"))
	assert.NilError(t, err, "Rename")
	expectFilesystemEvent(t, ch, Event{
		EventType: FileDeleted,
		Path:      repoRoot.UntypedJoin("parent"),
	})
	expectFilesystemEvent(t, ch, Event{
		EventType: FileAdded,
		Path:      repoRoot.UntypedJoin("new_parent"),
	})

	// Ensure we get an event when creating a file in renamed directory
	fooPath := repoRoot.UntypedJoin("new_parent", "child", "foo")
	err = fooPath.WriteFile([]byte("hello"), 0644)
	assert.NilError(t, err, "WriteFile")
	expectFilesystemEvent(t, ch, Event{
		EventType: FileAdded,
		Path:      fooPath,
	})
}

// TestFileWatchingRootRename tests that when the root is renamed,
// a delete event will be sent
//
// ✅ macOS
// ✅ Linux
// ❌ Windows - you cannot rename a watched folder (see https://github.com/fsnotify/fsnotify/issues/356)
func TestFileWatchingRootRename(t *testing.T) {
	if runtime.GOOS == "windows" {
		return
	}
	logger := hclog.Default()
	logger.SetLevel(hclog.Debug)
	oldRepoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())
	err := oldRepoRoot.UntypedJoin(".git").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	err = oldRepoRoot.UntypedJoin("node_modules", "some-dep").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	err = oldRepoRoot.UntypedJoin("parent", "child").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")

	// Directory layout:
	// <oldRepoRoot>/
	//	 .git/
	//   node_modules/
	//     some-dep/
	//   parent/
	//     child/

	watcher, err := GetPlatformSpecificBackend(logger)
	assert.NilError(t, err, "GetPlatformSpecificBackend")
	fw := New(logger, oldRepoRoot, watcher)
	err = fw.Start()
	assert.NilError(t, err, "fw.Start")

	// Add a client
	ch := make(chan Event, 1)
	c := &testClient{
		notify: ch,
	}
	fw.AddClient(c)
	expectedWatching := []turbopath.AbsoluteSystemPath{
		oldRepoRoot,
		oldRepoRoot.UntypedJoin("parent"),
		oldRepoRoot.UntypedJoin("parent", "child"),
	}
	expectWatching(t, c, expectedWatching)

	// Rename root folder during file watching
	newRepoRoot := oldRepoRoot.Dir().UntypedJoin("new_repo_root")
	err = oldRepoRoot.Rename(newRepoRoot)
	assert.NilError(t, err, "Rename")

	expectFilesystemEvent(t, ch, Event{
		EventType: FileDeleted,
		Path:      oldRepoRoot,
	})
	// We got the root delete event, no guarantees about what happens after that
}

// TestFileWatchSymlinkCreate tests that when a symlink is created,
// file watching will continue, and a file create event is sent.
// it also validates that new files in the symlinked directory will
// be watched, and raise events with the original path.
//
// ✅ macOS
// ✅ Linux
// ✅ Windows - requires admin permissions
func TestFileWatchSymlinkCreate(t *testing.T) {
	logger := hclog.Default()
	logger.SetLevel(hclog.Debug)
	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())
	err := repoRoot.UntypedJoin(".git").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	err = repoRoot.UntypedJoin("node_modules", "some-dep").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	err = repoRoot.UntypedJoin("parent", "child").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")

	// Directory layout:
	// <repoRoot>/
	//	 .git/
	//   node_modules/
	//     some-dep/
	//   parent/
	//     child/

	watcher, err := GetPlatformSpecificBackend(logger)
	assert.NilError(t, err, "GetPlatformSpecificBackend")
	fw := New(logger, repoRoot, watcher)
	err = fw.Start()
	assert.NilError(t, err, "fw.Start")

	// Add a client
	ch := make(chan Event, 1)
	c := &testClient{
		notify: ch,
	}
	fw.AddClient(c)
	expectedWatching := []turbopath.AbsoluteSystemPath{
		repoRoot,
		repoRoot.UntypedJoin("parent"),
		repoRoot.UntypedJoin("parent", "child"),
	}
	expectWatching(t, c, expectedWatching)

	// Create symlink during file watching
	symlinkPath := repoRoot.UntypedJoin("symlink")
	err = symlinkPath.Symlink(repoRoot.UntypedJoin("parent", "child").ToString())
	assert.NilError(t, err, "Symlink")
	expectFilesystemEvent(t, ch,
		Event{
			EventType: FileAdded,
			Path:      symlinkPath,
		},
	)

	// we expect that events in the symlinked directory will be raised with the original path
	symlinkSubfile := symlinkPath.UntypedJoin("symlink_subfile")
	err = symlinkSubfile.WriteFile([]byte("hello"), 0644)
	assert.NilError(t, err, "WriteFile")
	expectFilesystemEvent(t, ch,
		Event{
			EventType: FileAdded,
			Path:      repoRoot.UntypedJoin("parent", "child", "symlink_subfile"),
		},
	)
}

// TestFileWatchSymlinkDelete tests that when a symlink is deleted,
// file watching raises no events for the virtual path
//
// ✅ macOS
// ✅ Linux
// ✅ Windows - requires admin permissions
func TestFileWatchSymlinkDelete(t *testing.T) {
	logger := hclog.Default()
	logger.SetLevel(hclog.Debug)
	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())
	err := repoRoot.UntypedJoin(".git").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	err = repoRoot.UntypedJoin("node_modules", "some-dep").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	err = repoRoot.UntypedJoin("parent", "child").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	symlinkPath := repoRoot.UntypedJoin("symlink")
	err = symlinkPath.Symlink(repoRoot.UntypedJoin("parent", "child").ToString())
	assert.NilError(t, err, "Symlink")

	// Directory layout:
	// <repoRoot>/
	//	 .git/
	//   node_modules/
	//     some-dep/
	//   parent/
	//     child/
	//   symlink -> parent/child

	watcher, err := GetPlatformSpecificBackend(logger)
	assert.NilError(t, err, "GetPlatformSpecificBackend")
	fw := New(logger, repoRoot, watcher)
	err = fw.Start()
	assert.NilError(t, err, "fw.Start")

	// Add a client
	ch := make(chan Event, 1)
	c := &testClient{
		notify: ch,
	}
	fw.AddClient(c)
	expectedWatching := []turbopath.AbsoluteSystemPath{
		repoRoot,
		repoRoot.UntypedJoin("parent"),
		repoRoot.UntypedJoin("parent", "child"),
	}
	expectWatching(t, c, expectedWatching)

	// Delete symlink during file watching
	err = symlinkPath.Remove()
	assert.NilError(t, err, "Remove")
	expectFilesystemEvent(t, ch, Event{
		EventType: FileDeleted,
		Path:      symlinkPath,
	})
}

// TestFileWatchSymlinkRename tests that when a symlink is renamed,
// file watching raises a create event for the virtual path
//
// ✅ macOS
// ✅ Linux
// ❌ Windows - raises an event for creating the file
func TestFileWatchSymlinkRename(t *testing.T) {
	logger := hclog.Default()
	logger.SetLevel(hclog.Debug)
	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())
	err := repoRoot.UntypedJoin(".git").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	err = repoRoot.UntypedJoin("node_modules", "some-dep").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	err = repoRoot.UntypedJoin("parent", "child").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")
	symlinkPath := repoRoot.UntypedJoin("symlink")
	err = symlinkPath.Symlink(repoRoot.UntypedJoin("parent", "child").ToString())
	assert.NilError(t, err, "Symlink")

	// Directory layout:
	// <repoRoot>/
	//	 .git/
	//   node_modules/
	//     some-dep/
	//   parent/
	//     child/
	//   symlink -> parent/child

	watcher, err := GetPlatformSpecificBackend(logger)
	assert.NilError(t, err, "GetPlatformSpecificBackend")
	fw := New(logger, repoRoot, watcher)
	err = fw.Start()
	assert.NilError(t, err, "fw.Start")

	// Add a client
	ch := make(chan Event, 1)
	c := &testClient{
		notify: ch,
	}
	fw.AddClient(c)
	expectedWatching := []turbopath.AbsoluteSystemPath{
		repoRoot,
		repoRoot.UntypedJoin("parent"),
		repoRoot.UntypedJoin("parent", "child"),
	}
	expectWatching(t, c, expectedWatching)

	// Rename symlink during file watching
	newSymlinkPath := repoRoot.UntypedJoin("new_symlink")
	err = symlinkPath.Rename(newSymlinkPath)
	assert.NilError(t, err, "Rename")

	expectFilesystemEvent(t, ch, Event{
		EventType: FileDeleted,
		Path:      symlinkPath,
	})

	expectFilesystemEvent(t, ch, Event{
		EventType: FileAdded,
		Path:      newSymlinkPath,
	})

}

// TestFileWatchRootParentRename tests that when the parent directory of the root is renamed,
// file watching stops reporting events
//
// additionally, renmaing the root parent directory back to its original name should cause file watching
// to start reporting events again
//
// ✅ macOS
// ✅ Linux
// ❌ Windows
func TestFileWatchRootParentRename(t *testing.T) {
	if runtime.GOOS == "windows" {
		return
	}
	logger := hclog.Default()
	logger.SetLevel(hclog.Debug)

	parent := fs.AbsoluteSystemPathFromUpstream(t.TempDir())
	repoRoot := parent.UntypedJoin("repo")
	err := repoRoot.UntypedJoin(".git").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")

	// Directory layout:
	// <parent>/
	//   repo/
	//     .git/

	watcher, err := GetPlatformSpecificBackend(logger)
	assert.NilError(t, err, "GetPlatformSpecificBackend")
	fw := New(logger, repoRoot, watcher)
	err = fw.Start()
	assert.NilError(t, err, "fw.Start")

	// Add a client
	ch := make(chan Event, 1)
	c := &testClient{
		notify: ch,
	}
	fw.AddClient(c)
	expectedWatching := []turbopath.AbsoluteSystemPath{
		repoRoot,
	}
	expectWatching(t, c, expectedWatching)

	// Rename parent directory during file watching
	newRepoRoot := parent.UntypedJoin("new_repo")
	err = repoRoot.Rename(newRepoRoot)
	assert.NilError(t, err, "Rename")
	expectFilesystemEvent(t, ch, Event{
		EventType: FileDeleted,
		Path:      repoRoot,
	})
	// We got the root delete event, no guarantees about what happens after that
}

// TestFileWatchRootParentDelete tests that when the parent directory of the root is deleted
//
// ✅ macOS
// ✅ Linux
// ❌ Windows - L721 no create event is emitted
func TestFileWatchRootParentDelete(t *testing.T) {
	logger := hclog.Default()
	logger.SetLevel(hclog.Debug)

	parent := fs.AbsoluteSystemPathFromUpstream(t.TempDir())
	repoRoot := parent.UntypedJoin("repo")
	err := repoRoot.UntypedJoin(".git").MkdirAll(0775)
	assert.NilError(t, err, "MkdirAll")

	// Directory layout:
	// <parent>/
	//   repo/
	//     .git/

	watcher, err := GetPlatformSpecificBackend(logger)
	assert.NilError(t, err, "GetPlatformSpecificBackend")
	fw := New(logger, repoRoot, watcher)
	err = fw.Start()
	assert.NilError(t, err, "fw.Start")

	// Add a client
	ch := make(chan Event, 1)
	c := &testClient{
		notify: ch,
	}
	fw.AddClient(c)
	expectedWatching := []turbopath.AbsoluteSystemPath{
		repoRoot,
	}
	expectWatching(t, c, expectedWatching)

	// Delete parent directory during file watching
	err = parent.RemoveAll()
	assert.NilError(t, err, "RemoveAll")
	expectFilesystemEvent(t, ch, Event{
		EventType: FileDeleted,
		Path:      repoRoot,
	})
	// We got the root delete event, no guarantees about what happens after that
}
