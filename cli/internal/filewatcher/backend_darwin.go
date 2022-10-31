//go:build darwin
// +build darwin

package filewatcher

import (
	"fmt"
	"strings"
	"sync"
	"time"

	"github.com/pkg/errors"
	"github.com/yookoala/realpath"

	"github.com/fsnotify/fsevents"
	"github.com/hashicorp/go-hclog"
	"github.com/vercel/turbo/cli/internal/doublestar"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

type fseventsBackend struct {
	events  chan Event
	errors  chan error
	logger  hclog.Logger
	mu      sync.Mutex
	streams []*fsevents.EventStream
	closed  bool
}

func (f *fseventsBackend) Events() <-chan Event {
	return f.events
}

func (f *fseventsBackend) Errors() <-chan error {
	return f.errors
}

func (f *fseventsBackend) Close() error {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.closed {
		return ErrFilewatchingClosed
	}
	f.closed = true
	for _, stream := range f.streams {
		stream.Stop()
	}
	close(f.events)
	close(f.errors)
	return nil
}

func (f *fseventsBackend) Start() error {
	return nil
}

var (
	_eventLatency  = 10 * time.Millisecond
	_cookieTimeout = 500 * time.Millisecond
)

// AddRoot starts watching a new directory hierarchy. Events matching the provided excludePatterns
// will not be forwarded.
func (f *fseventsBackend) AddRoot(someRoot turbopath.AbsoluteSystemPath, excludePatterns ...string) error {
	// We need to resolve the real path to the hierarchy that we are going to watch
	realRoot, err := realpath.Realpath(someRoot.ToString())
	if err != nil {
		return err
	}
	root := fs.AbsoluteSystemPathFromUpstream(realRoot)
	dev, err := fsevents.DeviceForPath(root.ToString())
	if err != nil {
		return err
	}

	// Optimistically set up and start a stream, assuming the watch is still valid.
	s := &fsevents.EventStream{
		Paths:   []string{root.ToString()},
		Latency: _eventLatency,
		Device:  dev,
		Flags:   fsevents.FileEvents | fsevents.WatchRoot,
	}
	s.Start()
	events := s.Events

	// fsevents delivers events for all existing files first, so use a cookie to detect when we're ready for new events
	if err := waitForCookie(root, events, _cookieTimeout); err != nil {
		s.Stop()
		return err
	}

	// Now try to persist the stream.
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.closed {
		s.Stop()
		return ErrFilewatchingClosed
	}
	f.streams = append(f.streams, s)
	f.logger.Debug(fmt.Sprintf("watching root %v, excluding %v", root, excludePatterns))

	go func() {
		for evs := range events {
			for _, ev := range evs {
				isExcluded := false

				// 1. Ensure that we have a `/`-prefixed path from the event.
				var eventPath string
				if !strings.HasPrefix("/", ev.Path) {
					eventPath = "/" + ev.Path
				} else {
					eventPath = ev.Path
				}

				// 2. We're getting events from the real path, but we need to translate
				// back to the path we were provided since that's what the caller will
				// expect in terms of event paths.
				watchRootRelativePath := eventPath[len(realRoot):]
				processedEventPath := someRoot.UntypedJoin(watchRootRelativePath)

				// 3. Compare the event to all exclude patterns, short-circuit if we know
				// we are not watching this file.
				processedPathString := processedEventPath.ToString() // loop invariant
				for _, pattern := range excludePatterns {
					matches, err := doublestar.Match(pattern, processedPathString)
					if err != nil {
						f.errors <- err
					} else if matches {
						isExcluded = true
						break
					}
				}

				// 4. Report the file events we care about.
				if !isExcluded {
					f.events <- Event{
						Path:      processedEventPath,
						EventType: toFileEvent(ev.Flags),
					}
				}
			}
		}
	}()

	return nil
}

func waitForCookie(root turbopath.AbsoluteSystemPath, events <-chan []fsevents.Event, timeout time.Duration) error {
	// This cookie needs to be in a location that we're watching, and at this point we can't guarantee
	// what the root is, or if something like "node_modules/.cache/turbo" would make sense. As a compromise, ensure
	// that we clean it up even in the event of a failure.
	cookiePath := root.UntypedJoin(".turbo-cookie")
	if err := cookiePath.WriteFile([]byte("cookie"), 0755); err != nil {
		return err
	}
	expected := cookiePath.ToString()[1:] // trim leading slash
	if err := waitForEvent(events, expected, fsevents.ItemCreated, timeout); err != nil {
		// Attempt to not leave the cookie file lying around.
		// Ignore the error, since there's not much we can do with it.
		_ = cookiePath.Remove()
		return err
	}
	if err := cookiePath.Remove(); err != nil {
		return err
	}
	if err := waitForEvent(events, expected, fsevents.ItemRemoved, timeout); err != nil {
		return err
	}
	return nil
}

func waitForEvent(events <-chan []fsevents.Event, path string, flag fsevents.EventFlags, timeout time.Duration) error {
	ch := make(chan struct{})
	go func() {
		for evs := range events {
			for _, ev := range evs {
				if ev.Path == path && ev.Flags&flag != 0 {
					close(ch)
					return
				}
			}
		}
	}()
	select {
	case <-time.After(timeout):
		return errors.Wrap(ErrFailedToStart, "timed out waiting for initial fsevents cookie")
	case <-ch:
		return nil
	}
}

var _modifiedMask = fsevents.ItemModified | fsevents.ItemInodeMetaMod | fsevents.ItemFinderInfoMod | fsevents.ItemChangeOwner | fsevents.ItemXattrMod

func toFileEvent(flags fsevents.EventFlags) FileEvent {
	if flags&fsevents.ItemCreated != 0 {
		return FileAdded
	} else if flags&fsevents.ItemRemoved != 0 {
		return FileDeleted
	} else if flags&_modifiedMask != 0 {
		return FileModified
	} else if flags&fsevents.ItemRenamed != 0 {
		return FileRenamed
	} else if flags&fsevents.RootChanged != 0 {
		// count this as a delete, something affected the path to the root
		// of the stream
		return FileDeleted
	}
	return FileOther
}

// GetPlatformSpecificBackend returns a filewatching backend appropriate for the OS we are
// running on.
func GetPlatformSpecificBackend(logger hclog.Logger) (Backend, error) {
	return &fseventsBackend{
		events: make(chan Event),
		errors: make(chan error),
		logger: logger.Named("fsevents"),
	}, nil
}
