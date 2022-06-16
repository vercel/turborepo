// Package filewatcher is used to handle watching for file changes inside the monorepo
package filewatcher

import (
	"path/filepath"
	"strings"
	"sync"

	"github.com/hashicorp/go-hclog"
	"github.com/pkg/errors"
	"github.com/vercel/turborepo/cli/internal/fs"
)

// _ignores is the set of paths we exempt from file-watching
var _ignores = []string{".git", "node_modules"}

// FileWatchClient defines the callbacks used by the file watching loop.
// All methods are called from the same goroutine so they:
// 1) do not need synchronization
// 2) should minimize the work they are doing when called, if possible
type FileWatchClient interface {
	OnFileWatchEvent(ev Event)
	OnFileWatchError(err error)
	OnFileWatchClosed()
}

type FileEvent int

const (
	FileAdded FileEvent = iota
	FileDeleted
	FileModified
	FileRenamed
	FileOther
)

var (
	ErrFilewatchingClosed = errors.New("Close() has already been called for filewatching")
	ErrFailedToStart      = errors.New("filewatching failed to start")
)

type Event struct {
	Path      fs.AbsolutePath
	EventType FileEvent
}

type Backend interface {
	AddRoot(root fs.AbsolutePath, excludePatterns ...string) error
	Events() <-chan Event
	Errors() <-chan error
	Close() error
	Start() error
}

// FileWatcher handles watching all of the files in the monorepo.
// We currently ignore .git and top-level node_modules. We can revisit
// if necessary.
type FileWatcher struct {
	watcher Backend

	logger         hclog.Logger
	repoRoot       fs.AbsolutePath
	excludePattern string

	clientsMu sync.RWMutex
	clients   []FileWatchClient
	closed    bool
}

// New returns a new FileWatcher instance
func New(logger hclog.Logger, repoRoot fs.AbsolutePath, watcher Backend) *FileWatcher {
	excludes := make([]string, len(_ignores))
	for i, ignore := range _ignores {
		excludes[i] = filepath.ToSlash(repoRoot.Join(ignore).ToString() + "/**")
	}
	excludePattern := "{" + strings.Join(excludes, ",") + "}"
	return &FileWatcher{
		watcher:        watcher,
		logger:         logger,
		repoRoot:       repoRoot,
		excludePattern: excludePattern,
	}
}

func (fw *FileWatcher) Close() error {
	return fw.watcher.Close()
}

// Start recursively adds all directories from the repo root, redacts the excluded ones,
// then fires off a goroutine to respond to filesystem events
func (fw *FileWatcher) Start() error {
	if err := fw.watcher.AddRoot(fw.repoRoot, fw.excludePattern); err != nil {
		return err
	}
	if err := fw.watcher.Start(); err != nil {
		return err
	}
	go fw.watch()
	return nil
}

func (fw *FileWatcher) AddRoot(root fs.AbsolutePath, excludePatterns ...string) error {
	return fw.watcher.AddRoot(root, excludePatterns...)
}

// watch is the main file-watching loop. Watching is not recursive,
// so when new directories are added, they are manually recursively watched.
func (fw *FileWatcher) watch() {
outer:
	for {
		select {
		case ev, ok := <-fw.watcher.Events():
			if !ok {
				fw.logger.Info("Events channel closed. Exiting watch loop")
				break outer
			}
			fw.clientsMu.RLock()
			for _, client := range fw.clients {
				client.OnFileWatchEvent(ev)
			}
			fw.clientsMu.RUnlock()
		case err, ok := <-fw.watcher.Errors():
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
