// Package filewatcher is used to handle watching for file changes inside the monorepo
package filewatcher

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"

	"github.com/fsnotify/fsnotify"
	"github.com/hashicorp/go-hclog"
	"github.com/karrick/godirwalk"
	"github.com/pkg/errors"
	"github.com/vercel/turborepo/cli/internal/doublestar"
	"github.com/vercel/turborepo/cli/internal/fs"
)

// _ignores is the set of paths we exempt from file-watching
var _ignores = []string{".git", "node_modules"}

// FileWatchClient defines the callbacks used by the file watching loop.
// All methods are called from the same goroutine so they:
// 1) do not need synchronization
// 2) should minimize the work they are doing when called, if possible
type FileWatchClient interface {
	OnFileWatchEvent(ev fsnotify.Event)
	OnFileWatchError(err error)
	OnFileWatchClosed()
}

// FileWatcher handles watching all of the files in the monorepo.
// We currently ignore .git and top-level node_modules. We can revisit
// if necessary.
type FileWatcher struct {
	*fsnotify.Watcher

	logger         hclog.Logger
	repoRoot       fs.AbsolutePath
	excludePattern string

	clientsMu sync.RWMutex
	clients   []FileWatchClient
	closed    bool
}

// New returns a new FileWatcher instance
func New(logger hclog.Logger, repoRoot fs.AbsolutePath, watcher *fsnotify.Watcher) *FileWatcher {
	excludes := make([]string, len(_ignores))
	for i, ignore := range _ignores {
		excludes[i] = filepath.ToSlash(repoRoot.Join(ignore).ToString() + "/**")
	}
	excludePattern := "{" + strings.Join(excludes, ",") + "}"
	return &FileWatcher{
		Watcher:        watcher,
		logger:         logger,
		repoRoot:       repoRoot,
		excludePattern: excludePattern,
	}
}

// Start recursively adds all directories from the repo root, redacts the excluded ones,
// then fires off a goroutine to respond to filesystem events
func (fw *FileWatcher) Start() error {
	if err := fw.watchRecursively(fw.repoRoot); err != nil {
		return err
	}
	// Revoke the ignored directories, which are automatically added
	// because they are children of watched directories.
	for _, dir := range fw.WatchList() {
		excluded, err := doublestar.Match(fw.excludePattern, filepath.ToSlash(dir))
		if err != nil {
			return err
		}
		if excluded {
			if err := fw.Remove(dir); err != nil {
				fw.logger.Warn(fmt.Sprintf("failed to remove watch on %v: %v", dir, err))
			}
		}
	}
	go fw.watch()
	return nil
}

func (fw *FileWatcher) watchRecursively(root fs.AbsolutePath) error {
	err := fs.WalkMode(root.ToString(), func(name string, isDir bool, info os.FileMode) error {
		excluded, err := doublestar.Match(fw.excludePattern, filepath.ToSlash(name))
		if err != nil {
			return err
		}
		if excluded {
			return godirwalk.SkipThis
		}
		if info.IsDir() && (info&os.ModeSymlink == 0) {
			fw.logger.Debug(fmt.Sprintf("started watching %v", name))
			if err := fw.Add(name); err != nil {
				return errors.Wrapf(err, "failed adding watch to %v", name)
			}
		}
		return nil
	})
	if err != nil {
		return err
	}

	return nil
}

// onFileAdded helps up paper over cross-platform inconsistencies in fsnotify.
// Some fsnotify backends automatically add the contents of directories. Some do
// not. Adding a watch is idempotent, so anytime any file we care about gets added,
// watch it.
func (fw *FileWatcher) onFileAdded(name string) error {
	info, err := os.Lstat(name)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			// We can race with a file being added and removed. Ignore it
			return nil
		}
		return errors.Wrapf(err, "error checking lstat of new file %v", name)
	}
	if info.IsDir() {
		if err := fw.watchRecursively(fs.AbsolutePath(name)); err != nil {
			return errors.Wrapf(err, "failed recursive watch of %v", name)
		}
	} else {
		if err := fw.Add(name); err != nil {
			return errors.Wrapf(err, "failed adding watch to %v", name)
		}
	}
	return nil
}

// watch is the main file-watching loop. Watching is not recursive,
// so when new directories are added, they are manually recursively watched.
func (fw *FileWatcher) watch() {
outer:
	for {
		select {
		case ev, ok := <-fw.Watcher.Events:
			if !ok {
				fw.logger.Info("Events channel closed. Exiting watch loop")
				break outer
			}
			if ev.Op&fsnotify.Create != 0 {
				if err := fw.onFileAdded(ev.Name); err != nil {
					fw.logger.Warn(fmt.Sprintf("failed to handle adding %v: %v", ev.Name, err))
					continue outer
				}
			}
			fw.clientsMu.RLock()
			for _, client := range fw.clients {
				client.OnFileWatchEvent(ev)
			}
			fw.clientsMu.RUnlock()
		case err, ok := <-fw.Watcher.Errors:
			if !ok {
				fw.logger.Info("Errors channel closed. Exiting watch loop")
				break outer
			}
			fw.clientsMu.RLock()
			for _, client := range fw.clients {
				client.OnFileWatchError(err)
			}
			fw.clientsMu.RUnlock()
		}
	}
	fw.clientsMu.Lock()
	fw.closed = true
	for _, client := range fw.clients {
		client.OnFileWatchClosed()
	}
	fw.clientsMu.Unlock()
}

// AddClient registers a client for filesystem events
func (fw *FileWatcher) AddClient(client FileWatchClient) {
	fw.clientsMu.Lock()
	defer fw.clientsMu.Unlock()
	fw.clients = append(fw.clients, client)
	if fw.closed {
		client.OnFileWatchClosed()
	}
}
