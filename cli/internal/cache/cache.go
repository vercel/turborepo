// Package cache abstracts storing and fetching previously run tasks
package cache

import (
	"fmt"

	"github.com/vercel/turborepo/cli/internal/analytics"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/ui"
	"golang.org/x/sync/errgroup"
)

// Cache is abstracted way to cache/fetch previously run tasks
type Cache interface {
	// Fetch returns true if there is a cache it. It is expected to move files
	// into their correct position as a side effect
	Fetch(target string, hash string, files []string) (bool, []string, error)
	// Put caches files for a given hash
	Put(target string, hash string, duration int, files []string) error
	Clean(target string)
	CleanAll()
	Shutdown()
}

type cacheEvent struct {
	event string
	hash  string
}

// New creates a new cache
func New(config *config.Config, sink analytics.Sink) Cache {
	c := newSyncCache(config, false, sink)
	if config.Cache.Workers > 0 {
		return newAsyncCache(c, config)
	}
	return c
}

func newSyncCache(config *config.Config, remoteOnly bool, sink analytics.Sink) Cache {
	mplex := &cacheMultiplexer{}
	if config.Cache.Dir != "" && !remoteOnly {
		mplex.caches = append(mplex.caches, newFsCache(config, sink))
	}
	if (config.Token != "" && config.TeamId != "") || (config.Token != "" && config.TeamSlug != "") {
		fmt.Println(ui.Dim("• Remote computation caching enabled (experimental)"))
		mplex.caches = append(mplex.caches, newHTTPCache(config, sink))
	}
	if len(mplex.caches) == 0 {
		return nil
	} else if len(mplex.caches) == 1 {
		return mplex.caches[0] // Skip the extra layer of indirection
	}
	return mplex
}

// A cacheMultiplexer multiplexes several caches into one.
// Used when we have several active (eg. http, dir).
type cacheMultiplexer struct {
	caches []Cache
}

func (mplex cacheMultiplexer) Put(target string, key string, duration int, files []string) error {
	return mplex.storeUntil(target, key, duration, files, len(mplex.caches))
}

// storeUntil stores artifacts into higher priority caches than the given one.
// Used after artifact retrieval to ensure we have them in eg. the directory cache after
// downloading from the RPC cache.
// This is a little inefficient since we could write the file to plz-out then copy it to the dir cache,
// but it's hard to fix that without breaking the cache abstraction.
func (mplex cacheMultiplexer) storeUntil(target string, key string, duration int, outputGlobs []string, stopAt int) error {
	// Attempt to store on all caches simultaneously.
	g := &errgroup.Group{}
	for i, cache := range mplex.caches {
		if i == stopAt {
			break
		}
		c := cache
		g.Go(func() error {
			return c.Put(target, key, duration, outputGlobs)
		})
	}

	if err := g.Wait(); err != nil {
		return err
	}

	return nil
}

func (mplex cacheMultiplexer) Fetch(target string, key string, files []string) (bool, []string, error) {
	// Retrieve from caches sequentially; if we did them simultaneously we could
	// easily write the same file from two goroutines at once.
	for i, cache := range mplex.caches {
		if ok, actualFiles, err := cache.Fetch(target, key, files); ok {
			// Store this into other caches. We can ignore errors here because we know
			// we have previously successfully stored in a higher-priority cache, and so the overall
			// result is a success at fetching. Storing in lower-priority caches is an optimization.
			mplex.storeUntil(target, key, 0, actualFiles, i)
			return ok, actualFiles, err
		}
	}
	return false, files, nil
}

func (mplex cacheMultiplexer) Clean(target string) {
	for _, cache := range mplex.caches {
		cache.Clean(target)
	}
}

func (mplex cacheMultiplexer) CleanAll() {
	for _, cache := range mplex.caches {
		cache.CleanAll()
	}
}

func (mplex cacheMultiplexer) Shutdown() {
	for _, cache := range mplex.caches {
		cache.Shutdown()
	}
}
