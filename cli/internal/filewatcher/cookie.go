package filewatcher

import (
	"fmt"
	"os"
	"sync"
	"sync/atomic"
	"time"

	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// CookieWaiter is the interface used by clients that need to wait
// for a roundtrip through the filewatching API.
type CookieWaiter interface {
	WaitForCookie() error
}

var (
	// ErrCookieTimeout is returned when we did not see our cookie file within the given time constraints
	ErrCookieTimeout = errors.New("timed out waiting for cookie")
	// ErrCookieWatchingClosed is returned when the underlying filewatching has been closed.
	ErrCookieWatchingClosed = errors.New("filewatching has closed, cannot watch cookies")
)

// CookieJar is used for tracking roundtrips through the filesystem watching API
type CookieJar struct {
	timeout time.Duration
	dir     turbopath.AbsoluteSystemPath
	serial  uint64
	mu      sync.Mutex
	cookies map[turbopath.AbsoluteSystemPath]chan error
	closed  bool
}

// NewCookieJar returns a new instance of a CookieJar. There should only ever be a single
// instance live per cookieDir, since they expect to have full control over that directory.
func NewCookieJar(cookieDir turbopath.AbsoluteSystemPath, timeout time.Duration) (*CookieJar, error) {
	if err := cookieDir.RemoveAll(); err != nil {
		return nil, err
	}
	if err := cookieDir.MkdirAll(0775); err != nil {
		return nil, err
	}
	return &CookieJar{
		timeout: timeout,
		dir:     cookieDir,
		cookies: make(map[turbopath.AbsoluteSystemPath]chan error),
	}, nil
}

// removeAllCookiesWithError sends the error to every channel, closes every channel,
// and attempts to remove every cookie file. Must be called while the cj.mu is held.
// If the cookie jar is going to be reused afterwards, the cookies map must be reinitialized.
func (cj *CookieJar) removeAllCookiesWithError(err error) {
	for p, ch := range cj.cookies {
		_ = p.Remove()
		ch <- err
		close(ch)
	}
	// Drop all of the references so they can be cleaned up
	cj.cookies = nil
}

// OnFileWatchClosed handles the case where filewatching had to close for some reason
// We send an error to all of our cookies and stop accepting new ones.
func (cj *CookieJar) OnFileWatchClosed() {
	cj.mu.Lock()
	defer cj.mu.Unlock()
	cj.closed = true
	cj.removeAllCookiesWithError(ErrCookieWatchingClosed)

}

// OnFileWatchError handles when filewatching has encountered an error.
// In the error case, we remove all cookies and send them errors. We remain
// available for later cookies.
func (cj *CookieJar) OnFileWatchError(err error) {
	// We are now in an inconsistent state. Drop all of our cookies,
	// but we still allow new ones to be created
	cj.mu.Lock()
	defer cj.mu.Unlock()
	cj.removeAllCookiesWithError(err)
	cj.cookies = make(map[turbopath.AbsoluteSystemPath]chan error)
}

// OnFileWatchEvent determines if the specified event is relevant
// for cookie watching and notifies the appropriate cookie if so.
func (cj *CookieJar) OnFileWatchEvent(ev Event) {
	if ev.EventType == FileAdded {
		isCookie, err := fs.DirContainsPath(cj.dir.ToStringDuringMigration(), ev.Path.ToStringDuringMigration())
		if err != nil {
			cj.OnFileWatchError(errors.Wrapf(err, "failed to determine if path is a cookie: %v", ev.Path))
		} else if isCookie {
			cj.notifyCookie(ev.Path, nil)
		}
	}
}

// WaitForCookie touches a unique file, then waits for it to show up in filesystem notifications.
// This provides a theoretical bound on filesystem operations, although it's possible
// that underlying filewatch mechanisms don't respect this ordering.
func (cj *CookieJar) WaitForCookie() error {
	// we're only ever going to send a single error on the channel, add a buffer so that we never
	// block sending it.
	ch := make(chan error, 1)
	serial := atomic.AddUint64(&cj.serial, 1)
	cookiePath := cj.dir.UntypedJoin(fmt.Sprintf("%v.cookie", serial))
	cj.mu.Lock()
	if cj.closed {
		cj.mu.Unlock()
		return ErrCookieWatchingClosed
	}
	cj.cookies[cookiePath] = ch
	cj.mu.Unlock()
	if err := touchCookieFile(cookiePath); err != nil {
		cj.notifyCookie(cookiePath, err)
		return err
	}
	select {
	case <-time.After(cj.timeout):
		return ErrCookieTimeout
	case err, ok := <-ch:
		if !ok {
			// the channel closed without an error, we're all set
			return nil
		}
		// the channel didn't close, meaning we got some error.
		// We don't need to wait on channel close, it's going to be closed
		// immediately by whoever sent the error. Return the error directly
		return err
	}
}

func (cj *CookieJar) notifyCookie(cookie turbopath.AbsoluteSystemPath, err error) {
	cj.mu.Lock()
	ch, ok := cj.cookies[cookie]
	// delete is a no-op if the key doesn't exist
	delete(cj.cookies, cookie)
	cj.mu.Unlock()
	if ok {
		if err != nil {
			ch <- err
		}
		close(ch)
	}
}

func touchCookieFile(cookie turbopath.AbsoluteSystemPath) error {
	f, err := cookie.OpenFile(os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0700)
	if err != nil {
		return err
	}
	if err := f.Close(); err != nil {
		return err
	}
	return nil
}
