package filewatcher

import (
	"fmt"
	"sync"
	"testing"
	"time"

	"github.com/hashicorp/go-hclog"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/turbopath"
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

func expectWatching(t *testing.T, c *testClient, dirs []turbopath.AbsoluteSystemPath) {
	t.Helper()
	now := time.Now()
	filename := fmt.Sprintf("test-%v", now.UnixMilli())
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
