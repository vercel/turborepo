//go:build !darwin
// +build !darwin

package filewatcher

import (
	"fmt"
	"os"
	"path/filepath"
	"sync"

	"github.com/fsnotify/fsnotify"
	"github.com/hashicorp/go-hclog"
	"github.com/karrick/godirwalk"
	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/doublestar"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// watchAddMode is used to indicate whether watchRecursively should synthesize events
// for existing files.
type watchAddMode int

const (
	dontSynthesizeEvents watchAddMode = iota
	synthesizeEvents
)

type fsNotifyBackend struct {
	watcher *fsnotify.Watcher
	events  chan Event
	errors  chan error
	logger  hclog.Logger

	mu          sync.Mutex
	allExcludes []string
	closed      bool
}

func (f *fsNotifyBackend) Events() <-chan Event {
	return f.events
}

func (f *fsNotifyBackend) Errors() <-chan error {
	return f.errors
}

func (f *fsNotifyBackend) Close() error {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.closed {
		return ErrFilewatchingClosed
	}
	f.closed = true
	close(f.events)
	close(f.errors)
	if err := f.watcher.Close(); err != nil {
		return err
	}
	return nil
}

// onFileAdded helps up paper over cross-platform inconsistencies in fsnotify.
// Some fsnotify backends automatically add the contents of directories. Some do
// not. Adding a watch is idempotent, so anytime any file we care about gets added,
// watch it.
func (f *fsNotifyBackend) onFileAdded(name turbopath.AbsoluteSystemPath) error {
	info, err := name.Lstat()
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			// We can race with a file being added and removed. Ignore it
			return nil
		}
		return errors.Wrapf(err, "error checking lstat of new file %v", name)
	}
	if info.IsDir() {
		// If a directory has been added, we need to synthesize events for everything it contains
		if err := f.watchRecursively(name, []string{}, synthesizeEvents); err != nil {
			return errors.Wrapf(err, "failed recursive watch of %v", name)
		}
	} else {
		if err := f.watcher.Add(name.ToString()); err != nil {
			return errors.Wrapf(err, "failed adding watch to %v", name)
		}
	}
	return nil
}

func (f *fsNotifyBackend) watchRecursively(root turbopath.AbsoluteSystemPath, excludePatterns []string, addMode watchAddMode) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	err := fs.WalkMode(root.ToString(), func(name string, isDir bool, info os.FileMode) error {
		for _, excludePattern := range excludePatterns {
			excluded, err := doublestar.Match(excludePattern, filepath.ToSlash(name))
			if err != nil {
				return err
			}
			if excluded {
				return godirwalk.SkipThis
			}
		}
		if info.IsDir() && (info&os.ModeSymlink == 0) {
			if err := f.watcher.Add(name); err != nil {
				return errors.Wrapf(err, "failed adding watch to %v", name)
			}
			f.logger.Debug(fmt.Sprintf("watching directory %v", name))
		}
		if addMode == synthesizeEvents {
			f.events <- Event{
				Path:      fs.AbsoluteSystemPathFromUpstream(name),
				EventType: FileAdded,
			}
		}
		return nil
	})
	if err != nil {
		return err
	}
	f.allExcludes = append(f.allExcludes, excludePatterns...)

	return nil
}

func (f *fsNotifyBackend) watch() {
outer:
	for {
		select {
		case ev, ok := <-f.watcher.Events:
			if !ok {
				break outer
			}
			eventType := toFileEvent(ev.Op)
			path := fs.AbsoluteSystemPathFromUpstream(ev.Name)
			if eventType == FileAdded {
				if err := f.onFileAdded(path); err != nil {
					f.errors <- err
				}
			}
			f.events <- Event{
				Path:      path,
				EventType: eventType,
			}
		case err, ok := <-f.watcher.Errors:
			if !ok {
				break outer
			}
			f.errors <- err
		}
	}
}

var _modifiedMask = fsnotify.Chmod | fsnotify.Write

func toFileEvent(op fsnotify.Op) FileEvent {
	if op&fsnotify.Create != 0 {
		return FileAdded
	} else if op&fsnotify.Remove != 0 {
		return FileDeleted
	} else if op&_modifiedMask != 0 {
		return FileModified
	} else if op&fsnotify.Rename != 0 {
		return FileRenamed
	}
	return FileOther
}

func (f *fsNotifyBackend) Start() error {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.closed {
		return ErrFilewatchingClosed
	}
	for _, dir := range f.watcher.WatchList() {
		for _, excludePattern := range f.allExcludes {
			excluded, err := doublestar.Match(excludePattern, filepath.ToSlash(dir))
			if err != nil {
				return err
			}
			if excluded {
				if err := f.watcher.Remove(dir); err != nil {
					return err
				}
			}
		}
	}
	go f.watch()
	return nil
}

func (f *fsNotifyBackend) AddRoot(root turbopath.AbsoluteSystemPath, excludePatterns ...string) error {
	// We don't synthesize events for the initial watch
	return f.watchRecursively(root, excludePatterns, dontSynthesizeEvents)
}

// GetPlatformSpecificBackend returns a filewatching backend appropriate for the OS we are
// running on.
func GetPlatformSpecificBackend(logger hclog.Logger) (Backend, error) {
	watcher, err := fsnotify.NewWatcher()
	if err != nil {
		return nil, err
	}
	return &fsNotifyBackend{
		watcher: watcher,
		events:  make(chan Event),
		errors:  make(chan error),
		logger:  logger.Named("fsnotify"),
	}, nil
}
