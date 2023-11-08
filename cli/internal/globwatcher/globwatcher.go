package globwatcher

import (
	"errors"
	"fmt"
	"path/filepath"
	"sync"

	"github.com/hashicorp/go-hclog"
	"github.com/vercel/turbo/cli/internal/doublestar"
	"github.com/vercel/turbo/cli/internal/filewatcher"
	"github.com/vercel/turbo/cli/internal/fs/hash"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/util"
)

// ErrClosed is returned when attempting to get changed globs after glob watching has closed
var ErrClosed = errors.New("glob watching is closed")

type globs struct {
	Inclusions util.Set
	Exclusions util.Set
}

// GlobWatcher is used to track unchanged globs by hash. Once a glob registers a file change
// it is no longer tracked until a new hash requests it. Once all globs for a particular hash
// have changed, that hash is no longer tracked.
type GlobWatcher struct {
	logger       hclog.Logger
	repoRoot     turbopath.AbsoluteSystemPath
	cookieWaiter filewatcher.CookieWaiter

	mu         sync.RWMutex // protects field below
	hashGlobs  map[string]globs
	globStatus map[string]util.Set // glob -> hashes where this glob hasn't changed

	closed bool
}

// New returns a new GlobWatcher instance
func New(logger hclog.Logger, repoRoot turbopath.AbsoluteSystemPath, cookieWaiter filewatcher.CookieWaiter) *GlobWatcher {
	return &GlobWatcher{
		logger:       logger,
		repoRoot:     repoRoot,
		cookieWaiter: cookieWaiter,
		hashGlobs:    make(map[string]globs),
		globStatus:   make(map[string]util.Set),
	}
}

func (g *GlobWatcher) setClosed() {
	g.mu.Lock()
	g.closed = true
	g.mu.Unlock()
}

func (g *GlobWatcher) isClosed() bool {
	g.mu.RLock()
	defer g.mu.RUnlock()
	return g.closed
}

// WatchGlobs registers the given set of globs to be watched for changes and grouped
// under the given hash. This method pairs with GetChangedGlobs to determine which globs
// out of a set of candidates have changed since WatchGlobs was called for the same hash.
func (g *GlobWatcher) WatchGlobs(hash string, globsToWatch hash.TaskOutputs) error {
	if g.isClosed() {
		return ErrClosed
	}
	// Wait for a cookie here
	// that will ensure that we have seen all filesystem writes
	// *by the calling client*. Other tasks _could_ write to the
	// same output directories, however we are relying on task
	// execution dependencies to prevent that.
	if err := g.cookieWaiter.WaitForCookie(); err != nil {
		return err
	}
	g.mu.Lock()
	defer g.mu.Unlock()
	g.hashGlobs[hash] = globs{
		Inclusions: util.SetFromStrings(globsToWatch.Inclusions),
		Exclusions: util.SetFromStrings(globsToWatch.Exclusions),
	}

	for _, glob := range globsToWatch.Inclusions {
		existing, ok := g.globStatus[glob]
		if !ok {
			existing = make(util.Set)
		}
		existing.Add(hash)
		g.globStatus[glob] = existing
	}
	return nil
}

// GetChangedGlobs returns the subset of the given candidates that we are not currently
// tracking as "unchanged".
func (g *GlobWatcher) GetChangedGlobs(hash string, candidates []string) ([]string, error) {
	if g.isClosed() {
		// If filewatching has crashed, return all candidates as changed.
		return candidates, nil
	}
	// Wait for a cookie here
	// that will ensure that we have seen all filesystem writes
	// *by the calling client*. Other tasks _could_ write to the
	// same output directories, however we are relying on task
	// execution dependencies to prevent that.
	if err := g.cookieWaiter.WaitForCookie(); err != nil {
		return nil, err
	}
	// hashGlobs tracks all of the unchanged globs for a given hash
	// If hashGlobs doesn't have our hash, either everything has changed,
	// or we were never tracking it. Either way, consider all the candidates
	// to be changed globs.
	g.mu.RLock()
	defer g.mu.RUnlock()
	globsToCheck, ok := g.hashGlobs[hash]
	if !ok {
		return candidates, nil
	}
	allGlobs := util.SetFromStrings(candidates)
	diff := allGlobs.Difference(globsToCheck.Inclusions)

	return diff.UnsafeListOfStrings(), nil
}

// OnFileWatchEvent implements FileWatchClient.OnFileWatchEvent
// On a file change, check if we have a glob that matches this file. Invalidate
// any matching globs, and remove them from the set of unchanged globs for the corresponding
// hashes. If this is the last glob for a hash, remove the hash from being tracked.
func (g *GlobWatcher) OnFileWatchEvent(ev filewatcher.Event) {
	// At this point, we don't care what the Op is, any Op represents a change
	// that should invalidate matching globs
	g.logger.Trace(fmt.Sprintf("Got fsnotify event %v", ev))
	absolutePath := ev.Path
	repoRelativePath, err := g.repoRoot.RelativePathString(absolutePath.ToStringDuringMigration())
	if err != nil {
		g.logger.Debug(fmt.Sprintf("could not get relative path from %v to %v: %v", g.repoRoot, absolutePath, err))
		return
	}
	g.mu.Lock()
	defer g.mu.Unlock()
	for glob, hashStatus := range g.globStatus {
		matches, err := doublestar.Match(glob, filepath.ToSlash(repoRelativePath))
		if err != nil {
			g.logger.Error(fmt.Sprintf("failed to check path %v against glob %v: %v", repoRelativePath, glob, err))
			continue
		}
		// If this glob matches, we know that it has changed for every hash that included this glob
		// and is not excluded by a hash's exclusion globs.
		// So, we can delete this glob from every hash tracking it as well as stop watching this glob.
		// To stop watching, we unref each of the directories corresponding to this glob.
		if matches {
			for hashUntyped := range hashStatus {
				hash := hashUntyped.(string)
				hashGlobs, ok := g.hashGlobs[hash]

				if !ok {
					g.logger.Warn(fmt.Sprintf("failed to find hash %v referenced from glob %v", hash, glob))
					continue
				}

				isExcluded := false
				// Check if we've excluded this path by going through exclusion globs
				for exclusionGlob := range hashGlobs.Exclusions {
					matches, err := doublestar.Match(exclusionGlob.(string), filepath.ToSlash(repoRelativePath))
					if err != nil {
						g.logger.Error(fmt.Sprintf("failed to check path %v against glob %v: %v", repoRelativePath, glob, err))
						continue
					}

					if matches {
						isExcluded = true
						break
					}
				}

				// If we have excluded this path, then we skip it
				if isExcluded {
					continue
				}

				// We delete hash from the globStatus entry
				g.globStatus[glob].Delete(hash)

				// If we've deleted the last hash for a glob in globStatus, delete the whole glob entry
				if len(g.globStatus[glob]) == 0 {
					delete(g.globStatus, glob)
				}

				hashGlobs.Inclusions.Delete(glob)
				// If we've deleted the last glob for a hash, delete the whole hash entry
				if hashGlobs.Inclusions.Len() == 0 {
					delete(g.hashGlobs, hash)
				}
			}
		}
	}
}

// OnFileWatchError implements FileWatchClient.OnFileWatchError
func (g *GlobWatcher) OnFileWatchError(err error) {
	g.logger.Error(fmt.Sprintf("file watching received an error: %v", err))
}

// OnFileWatchClosed implements FileWatchClient.OnFileWatchClosed
func (g *GlobWatcher) OnFileWatchClosed() {
	g.setClosed()
	g.logger.Warn("GlobWatching is closing due to file watching closing")
}
