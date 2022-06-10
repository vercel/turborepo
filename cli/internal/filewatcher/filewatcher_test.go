package filewatcher

import (
	"runtime"
	"sync"
	"testing"
	"time"

	"github.com/fsnotify/fsnotify"
	"github.com/hashicorp/go-hclog"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/util"
	"gotest.tools/v3/assert"
)

type testClient struct {
	mu           sync.Mutex
	createEvents []fsnotify.Event
	notify       chan<- struct{}
}

type helper interface {
	Helper()
}

func (c *testClient) OnFileWatchEvent(ev fsnotify.Event) {
	if ev.Op&fsnotify.Create != 0 {
		c.mu.Lock()
		defer c.mu.Unlock()
		c.createEvents = append(c.createEvents, ev)
		c.notify <- struct{}{}
	}
}

func (c *testClient) OnFileWatchError(err error) {}

func (c *testClient) OnFileWatchClosed() {}

func assertSameSet(t *testing.T, gotSlice []string, wantSlice []string) {
	// mark this method as a helper
	var tt interface{} = t
	helper, ok := tt.(helper)
	if ok {
		helper.Helper()
	}
	got := util.SetFromStrings(gotSlice)
	want := util.SetFromStrings(wantSlice)
	extra := got.Difference(want)
	missing := want.Difference(got)
	if extra.Len() > 0 {
		t.Errorf("found extra elements: %v", extra.UnsafeListOfStrings())
	}
	if missing.Len() > 0 {
		t.Errorf("missing expected elements: %v", missing.UnsafeListOfStrings())
	}
}

func expectFilesystemEvent(t *testing.T, ch <-chan struct{}) {
	// mark this method as a helper
	t.Helper()
	select {
	case <-ch:
		return
	case <-time.After(1 * time.Second):
		t.Error("Timed out waiting for filesystem event")
	}
}

func expectNoFilesystemEvent(t *testing.T, ch <-chan struct{}) {
	// mark this method as a helper
	t.Helper()
	select {
	case ev, ok := <-ch:
		if ok {
			t.Errorf("got unexpected filesystem event %v", ev)
		} else {
			t.Error("filewatching closed unexpectedly")
		}
	case <-time.After(100 * time.Millisecond):
		return
	}
}

func TestFileWatching(t *testing.T) {
	logger := hclog.Default()
	repoRoot := fs.AbsolutePathFromUpstream(t.TempDir())
	err := repoRoot.Join(".git").MkdirAll()
	assert.NilError(t, err, "MkdirAll")
	err = repoRoot.Join("node_modules", "some-dep").MkdirAll()
	assert.NilError(t, err, "MkdirAll")
	err = repoRoot.Join("parent", "child").MkdirAll()
	assert.NilError(t, err, "MkdirAll")
	err = repoRoot.Join("parent", "sibling").MkdirAll()
	assert.NilError(t, err, "MkdirAll")

	// Directory layout:
	// <repoRoot>/
	//	 .git/
	//   node_modules/
	//     some-dep/
	//   parent/
	//     child/
	//     sibling/

	watcher, err := fsnotify.NewWatcher()
	assert.NilError(t, err, "NewWatcher")
	fw := New(logger, repoRoot, watcher)
	err = fw.Start()
	assert.NilError(t, err, "watchRecursively")
	expectedWatching := []string{
		repoRoot.ToString(),
		repoRoot.Join("parent").ToString(),
		repoRoot.Join("parent", "child").ToString(),
		repoRoot.Join("parent", "sibling").ToString(),
	}
	watching := fw.WatchList()
	assertSameSet(t, watching, expectedWatching)

	// Add a client
	ch := make(chan struct{}, 1)
	c := &testClient{
		notify: ch,
	}
	fw.AddClient(c)
	go fw.watch()

	fooPath := repoRoot.Join("parent", "child", "foo")
	err = fooPath.WriteFile([]byte("hello"), 0644)
	assert.NilError(t, err, "WriteFile")
	expectFilesystemEvent(t, ch)
	expectedEvent := fsnotify.Event{
		Op:   fsnotify.Create,
		Name: fooPath.ToString(),
	}
	c.mu.Lock()
	got := c.createEvents[len(c.createEvents)-1]
	c.mu.Unlock()
	assert.DeepEqual(t, got, expectedEvent)
	// Windows doesn't watch individual files, only directories
	if runtime.GOOS != "windows" {
		expectedWatching = append(expectedWatching, fooPath.ToString())
	}
	watching = fw.WatchList()
	assertSameSet(t, watching, expectedWatching)

	deepPath := repoRoot.Join("parent", "sibling", "deep", "path")
	err = deepPath.MkdirAll()
	assert.NilError(t, err, "MkdirAll")
	// We'll catch an event for "deep", but not "deep/path" since
	// we don't have a recursive watch
	expectFilesystemEvent(t, ch)

	expectedWatching = append(expectedWatching, deepPath.ToString(), repoRoot.Join("parent", "sibling", "deep").ToString())
	watching = fw.WatchList()
	assertSameSet(t, watching, expectedWatching)

	gitFilePath := repoRoot.Join(".git", "git-file")
	err = gitFilePath.WriteFile([]byte("nope"), 0644)
	assert.NilError(t, err, "WriteFile")
	expectNoFilesystemEvent(t, ch)

	// No change in watchlist
	watching = fw.WatchList()
	assertSameSet(t, watching, expectedWatching)
}
