// Package filewatcher is used to handle watching for file changes inside the monorepo
package filewatcher

import (
	"path/filepath"
	"strings"
	"sync"

	"github.com/hashicorp/go-hclog"
	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/turbopath"
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

// FileEvent is an enum covering the kinds of things that can happen
// to files that we might be interested in
type FileEvent int

const (
	// FileAdded - this is a new file
	FileAdded FileEvent = iota + 1
	// FileDeleted - this file has been removed
	FileDeleted
	// FileModified - this file has been changed in some way
	FileModified
	// FileRenamed - a file's name has changed
	FileRenamed
	// FileOther - some other backend-specific event has happened
	FileOther
)

var (
	// ErrFilewatchingClosed is returned when filewatching has been closed
	ErrFilewatchingClosed = errors.New("Close() has already been called for filewatching")
	// ErrFailedToStart is returned when filewatching fails to start up
	ErrFailedToStart = errors.New("filewatching failed to start")
)

// Event is the backend-independent information about a file change
type Event struct {
	Path      turbopath.AbsoluteSystemPath
	EventType FileEvent
}

// Backend is the interface that describes what an underlying filesystem watching backend
// must provide.
type Backend interface {
	AddRoot(root turbopath.AbsoluteSystemPath, excludePatterns ...string) error
	Events() <-chan Event
	Errors() <-chan error
	Close() error
	Start() error
}

// FileWatcher handles watching all of the files in the monorepo.
// We currently ignore .git and top-level node_modules. We can revisit
// if necessary.
type FileWatcher struct {
	backend Backend

	logger         hclog.Logger
	repoRoot       turbopath.AbsoluteSystemPath
	excludePattern string

	clientsMu sync.RWMutex
	clients   []FileWatchClient
	closed    bool
}

// New returns a new FileWatcher instance
func New(logger hclog.Logger, repoRoot turbopath.AbsoluteSystemPath, backend Backend) *FileWatcher {
	excludes := make([]string, len(_ignores))
	for i, ignore := range _ignores {
		excludes[i] = filepath.ToSlash(repoRoot.UntypedJoin(ignore).ToString() + "/**")
	}
	excludePattern := "{" + strings.Join(excludes, ",") + "}"
	return &FileWatcher{
		backend:        backend,
		logger:         logger,
		repoRoot:       repoRoot,
		excludePattern: excludePattern,
	}
}

// Close shuts down filewatching
func (fw *FileWatcher) Close() error {
	return fw.backend.Close()
}

// Start recursively adds all directories from the repo root, redacts the excluded ones,
// then fires off a goroutine to respond to filesystem events
func (fw *FileWatcher) Start() error {
	if err := fw.backend.AddRoot(fw.repoRoot, fw.excludePattern); err != nil {
		return err
	}
	if err := fw.backend.Start(); err != nil {
		return err
	}
	go fw.watch()
	return nil
}

// AddRoot registers the root a filesystem hierarchy to be watched for changes. Events are *not*
// fired for existing files when AddRoot is called, only for subsequent changes.
// NOTE: if it appears helpful, we could change this behavior so that we provide a stream of initial
// events.
func (fw *FileWatcher) AddRoot(root turbopath.AbsoluteSystemPath, excludePatterns ...string) error {
	return fw.backend.AddRoot(root, excludePatterns...)
}

// watch is the main file-watching loop. Watching is not recursive,
// so when new directories are added, they are manually recursively watched.
func (fw *FileWatcher) watch() {
outer:
	for {
		select {
		case ev, ok := <-fw.backend.Events():
			if !ok {
				fw.logger.Info("Events channel closed. Exiting watch loop")
				break outer
			}
			fw.clientsMu.RLock()
			for _, client := range fw.clients {
				client.OnFileWatchEvent(ev)
			}
			fw.clientsMu.RUnlock()
		case err, ok := <-fw.backend.Errors():
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
