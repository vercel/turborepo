package filewatcher

import (
	"testing"
	"time"

	"github.com/hashicorp/go-hclog"
	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/fs"
	"gotest.tools/v3/assert"
)

func TestWaitForCookie(t *testing.T) {
	logger := hclog.Default()
	cookieDir := fs.AbsoluteSystemPathFromUpstream(t.TempDir())
	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())

	jar, err := NewCookieJar(cookieDir, 5*time.Second)
	assert.NilError(t, err, "NewCookieJar")

	watcher, err := GetPlatformSpecificBackend(logger)
	assert.NilError(t, err, "NewWatcher")
	fw := New(logger, repoRoot, watcher)
	err = fw.Start()
	assert.NilError(t, err, "Start")
	fw.AddClient(jar)
	err = fw.AddRoot(cookieDir)
	assert.NilError(t, err, "Add")

	err = jar.WaitForCookie()
	assert.NilError(t, err, "failed to roundtrip cookie")
}

func TestWaitForCookieAfterClose(t *testing.T) {
	logger := hclog.Default()
	cookieDir := fs.AbsoluteSystemPathFromUpstream(t.TempDir())
	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())

	jar, err := NewCookieJar(cookieDir, 5*time.Second)
	assert.NilError(t, err, "NewCookieJar")

	watcher, err := GetPlatformSpecificBackend(logger)
	assert.NilError(t, err, "NewWatcher")
	fw := New(logger, repoRoot, watcher)
	err = fw.Start()
	assert.NilError(t, err, "Start")
	fw.AddClient(jar)
	err = fw.AddRoot(cookieDir)
	assert.NilError(t, err, "Add")

	err = fw.Close()
	assert.NilError(t, err, "Close")
	err = jar.WaitForCookie()
	assert.ErrorIs(t, err, ErrCookieWatchingClosed)
}

func TestWaitForCookieTimeout(t *testing.T) {
	logger := hclog.Default()
	cookieDir := fs.AbsoluteSystemPathFromUpstream(t.TempDir())
	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())

	jar, err := NewCookieJar(cookieDir, 10*time.Millisecond)
	assert.NilError(t, err, "NewCookieJar")

	watcher, err := GetPlatformSpecificBackend(logger)
	assert.NilError(t, err, "NewWatcher")
	fw := New(logger, repoRoot, watcher)
	err = fw.Start()
	assert.NilError(t, err, "Start")
	fw.AddClient(jar)

	// NOTE: don't call fw.Add here so that no file event gets delivered

	err = jar.WaitForCookie()
	assert.ErrorIs(t, err, ErrCookieTimeout)
}

func TestWaitForCookieWithError(t *testing.T) {
	logger := hclog.Default()
	cookieDir := fs.AbsoluteSystemPathFromUpstream(t.TempDir())
	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())

	jar, err := NewCookieJar(cookieDir, 10*time.Second)
	assert.NilError(t, err, "NewCookieJar")

	watcher, err := GetPlatformSpecificBackend(logger)
	assert.NilError(t, err, "NewWatcher")
	fw := New(logger, repoRoot, watcher)
	err = fw.Start()
	assert.NilError(t, err, "Start")
	fw.AddClient(jar)

	// NOTE: don't call fw.Add here so that no file event gets delivered
	myErr := errors.New("an error")
	ch := make(chan error)
	go func() {
		if err := jar.WaitForCookie(); err != nil {
			ch <- err
		}
		close(ch)
	}()
	// wait for the cookie to be registered in the jar
	for {
		found := false
		jar.mu.Lock()
		if len(jar.cookies) == 1 {
			found = true
		}
		jar.mu.Unlock()
		if found {
			break
		}
		<-time.After(10 * time.Millisecond)
	}
	jar.OnFileWatchError(myErr)

	err, ok := <-ch
	if !ok {
		t.Error("expected to get an error from cookie watching")
	}
	assert.ErrorIs(t, err, myErr)

	// ensure waiting for a new cookie still works.
	// Add the filewatch to allow cookies work normally
	err = fw.AddRoot(cookieDir)
	assert.NilError(t, err, "Add")

	err = jar.WaitForCookie()
	assert.NilError(t, err, "WaitForCookie")
}
