//go:build darwin
// +build darwin

package filewatcher

import (
	"fmt"
	"path/filepath"
	"sync"
	"time"

	"github.com/pkg/errors"
	"github.com/yookoala/realpath"

	"github.com/fsnotify/fsevents"
	"github.com/hashicorp/go-hclog"
	"github.com/vercel/turborepo/cli/internal/doublestar"
	"github.com/vercel/turborepo/cli/internal/fs"
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

func (f *fseventsBackend) AddRoot(someRoot fs.AbsolutePath, excludePatterns ...string) error {
	// We need to resolve the real path to the hierarchy that we are going to watch
	realRoot, err := realpath.Realpath(someRoot.ToString())
	if err != nil {
		return err
	}
	root := fs.AbsolutePathFromUpstream(realRoot)
	dev, err := fsevents.DeviceForPath(root.ToString())
	if err != nil {
		return err
	}
	//events := make(chan []fsevents.Event)
	s := &fsevents.EventStream{
		Paths:   []string{root.ToString()},
		Latency: _eventLatency,
		Device:  dev,
		Flags:   fsevents.FileEvents | fsevents.WatchRoot,
	}
	s.Start() // called with the lock so it doesn't race with a call to Close()
	events := s.Events
	f.logger.Debug(fmt.Sprintf("watching root %v, excluding %v", root, excludePatterns))
	// fsevents delivers events for all existing files first, so use a cookie to detect when we're ready for new events
	if err := waitForCookie(root, events, _cookieTimeout); err != nil {
		s.Stop()
		return err
	}
	f.mu.Lock()
	if f.closed {
		s.Stop()
		f.mu.Unlock()
		return ErrFilewatchingClosed
	}
	f.streams = append(f.streams, s)
	f.mu.Unlock()
	go func() {
		for evs := range events {
			for _, ev := range evs {
				isExcluded := false
				eventPath := "/" + ev.Path
				// Typically this will be false, but in the case of an event
				// at the root of the stream, it will have the leading '/'
				if ev.Path[0] == '/' {
					eventPath = ev.Path
				}
				// we're getting events from the real path, but we need to translate
				// back to the path we were provided, since that's what the caller will
				// expect in terms of event paths.
				eventPath = someRoot.ToString() + eventPath[len(realRoot):]
				for _, pattern := range excludePatterns {
					matches, err := doublestar.Match(pattern, filepath.ToSlash(eventPath))
					if err != nil {
						f.errors <- err
					} else if matches {
						isExcluded = true
						break
					}
				}
				if !isExcluded {
					f.events <- Event{
						Path:      fs.AbsolutePathFromUpstream(eventPath),
						EventType: toFileEvent(ev.Flags),
					}
				}
			}
		}
	}()
	return nil
}

func waitForCookie(root fs.AbsolutePath, events <-chan []fsevents.Event, timeout time.Duration) error {
	cookiePath := root.Join(".turbo-cookie")
	if err := cookiePath.WriteFile([]byte("cookie"), 0755); err != nil {
		return err
	}
	expected := cookiePath.ToString()[1:] // trim leading slash
	if err := waitForEvent(events, expected, fsevents.ItemCreated, timeout); err != nil {
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

func GetPlatformSpecificWatcher(logger hclog.Logger) (*fseventsBackend, error) {
	return &fseventsBackend{
		events: make(chan Event),
		errors: make(chan error),
		logger: logger.Named("fsevents"),
	}, nil
}
