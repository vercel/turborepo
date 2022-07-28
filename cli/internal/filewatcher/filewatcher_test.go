package filewatcher

import (
	"fmt"
	"sync"
	"testing"
	"time"

	"github.com/hashicorp/go-hclog"
	"github.com/vercel/turborepo/cli/internal/fs"
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
		c.notify <- ev
	}
}

func (c *testClient) OnFileWatchError(err error) {}

func (c *testClient) OnFileWatchClosed() {}

func expectFilesystemEvent(t *testing.T, ch <-chan Event, expected Event) {
	// mark this method as a helper
	t.Helper()
	timeout := time.After(1 * time.Second)
	for {
		select {
		case ev := <-ch:
			t.Logf("got event %v", ev)
			if ev.Path == expected.Path && ev.EventType == expected.EventType {
				return
			}
		case <-timeout:
			t.Errorf("Timed out waiting for filesystem event at %v", expected.Path)
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

func expectWatching(t *testing.T, c *testClient, dirs []fs.AbsolutePath) {
	t.Helper()
	now := time.Now()
	filename := fmt.Sprintf("test-%v", now.UnixMilli())
	for _, dir := range dirs {
		file := dir.Join(filename)
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
	expectedWatching := []fs.AbsolutePath{
		repoRoot,
		repoRoot.Join("parent"),
		repoRoot.Join("parent", "child"),
		repoRoot.Join("parent", "sibling"),
	}
	expectWatching(t, c, expectedWatching)

	fooPath := repoRoot.Join("parent", "child", "foo")
	err = fooPath.WriteFile([]byte("hello"), 0644)
	assert.NilError(t, err, "WriteFile")
	expectFilesystemEvent(t, ch, Event{
		EventType: FileAdded,
		Path:      fooPath,
	})

	deepPath := repoRoot.Join("parent", "sibling", "deep", "path")
	err = deepPath.MkdirAll()
	assert.NilError(t, err, "MkdirAll")
	// We'll catch an event for "deep", but not "deep/path" since
	// we don't have a recursive watch
	expectFilesystemEvent(t, ch, Event{
		Path:      repoRoot.Join("parent", "sibling", "deep"),
		EventType: FileAdded,
	})
	expectFilesystemEvent(t, ch, Event{
		Path:      repoRoot.Join("parent", "sibling", "deep", "path"),
		EventType: FileAdded,
	})
	expectedWatching = append(expectedWatching, deepPath, repoRoot.Join("parent", "sibling", "deep"))
	expectWatching(t, c, expectedWatching)

	gitFilePath := repoRoot.Join(".git", "git-file")
	err = gitFilePath.WriteFile([]byte("nope"), 0644)
	assert.NilError(t, err, "WriteFile")
	expectNoFilesystemEvent(t, ch)
}
