// Package cache abstracts storing and fetching previously run tasks
//
// Adapted from https://github.com/thought-machine/please
// Copyright Thought Machine, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
package cache

import (
	"errors"
	"fmt"
	"sync"

	"github.com/vercel/turborepo/cli/internal/analytics"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"
	"golang.org/x/sync/errgroup"
)

// Cache is abstracted way to cache/fetch previously run tasks
type Cache interface {
	// Fetch returns true if there is a cache it. It is expected to move files
	// into their correct position as a side effect
	Fetch(target string, hash string, files []string) (bool, []string, int, error)
	// Put caches files for a given hash
	Put(target string, hash string, duration int, files []string) error
	Clean(target string)
	CleanAll()
	Shutdown()
}

const cacheEventHit = "HIT"
const cacheEventMiss = "MISS"

type CacheEvent struct {
	Source   string `mapstructure:"source"`
	Event    string `mapstructure:"event"`
	Hash     string `mapstructure:"hash"`
	Duration int    `mapstructure:"duration"`
}

// OnCacheRemoved defines a callback that the cache system calls if a particular cache
// needs to be removed. In practice, this happens when Remote Caching has been disabled
// the but CLI continues to try to use it.
type OnCacheRemoved = func(cache Cache, err error)

// ErrNoCachesEnabled is returned when both the filesystem and http cache are unavailable
var ErrNoCachesEnabled = errors.New("no caches are enabled")

// New creates a new cache
func New(config *config.Config, remoteOnly bool, recorder analytics.Recorder, onCacheRemoved OnCacheRemoved) (Cache, error) {
	c, err := newSyncCache(config, remoteOnly, recorder, onCacheRemoved)
	if err != nil {
		return nil, err
	}
	if config.Cache.Workers > 0 {
		return newAsyncCache(c, config), nil
	}
	return c, nil
}

func newSyncCache(config *config.Config, remoteOnly bool, recorder analytics.Recorder, onCacheRemoved OnCacheRemoved) (Cache, error) {
	mplex := &cacheMultiplexer{
		onCacheRemoved: onCacheRemoved,
	}
	if config.Cache.Dir != "" && !remoteOnly {
		mplex.caches = append(mplex.caches, newFsCache(config, recorder))
	}
	if config.IsLoggedIn() {
		fmt.Println(ui.Dim("• Remote computation caching enabled (experimental)"))
		mplex.caches = append(mplex.caches, newHTTPCache(config, recorder))
	}
	if len(mplex.caches) == 0 {
		return nil, ErrNoCachesEnabled
	} else if len(mplex.caches) == 1 {
		return mplex.caches[0], nil // Skip the extra layer of indirection
	}
	return mplex, nil
}

// A cacheMultiplexer multiplexes several caches into one.
// Used when we have several active (eg. http, dir).
type cacheMultiplexer struct {
	caches         []Cache
	mu             sync.RWMutex
	onCacheRemoved OnCacheRemoved
}

func (mplex *cacheMultiplexer) Put(target string, key string, duration int, files []string) error {
	return mplex.storeUntil(target, key, duration, files, len(mplex.caches))
}

type cacheRemoval struct {
	cache Cache
	err   *util.CacheDisabledError
}

// storeUntil stores artifacts into higher priority caches than the given one.
// Used after artifact retrieval to ensure we have them in eg. the directory cache after
// downloading from the RPC cache.
func (mplex *cacheMultiplexer) storeUntil(target string, key string, duration int, outputGlobs []string, stopAt int) error {
	// Attempt to store on all caches simultaneously.
	toRemove := make([]*cacheRemoval, stopAt)
	g := &errgroup.Group{}
	mplex.mu.RLock()
	for i, cache := range mplex.caches {
		if i == stopAt {
			break
		}
		c := cache
		i := i
		g.Go(func() error {
			err := c.Put(target, key, duration, outputGlobs)
			if err != nil {
				cd := &util.CacheDisabledError{}
				if errors.As(err, &cd) {
					toRemove[i] = &cacheRemoval{
						cache: c,
						err:   cd,
					}
					// we don't want this to cancel other cache actions
					return nil
				}
				return err
			}
			return nil
		})
	}
	mplex.mu.RUnlock()

	if err := g.Wait(); err != nil {
		return err
	}

	for _, removal := range toRemove {
		if removal != nil {
			mplex.removeCache(removal)
		}
	}
	return nil
}

// removeCache takes a requested removal and tries to actually remove it. However,
// multiple requests could result in concurrent requests to remove the same cache.
// Let one of them win and propagate the error, the rest will no-op.
func (mplex *cacheMultiplexer) removeCache(removal *cacheRemoval) {
	mplex.mu.Lock()
	defer mplex.mu.Unlock()
	for i, cache := range mplex.caches {
		if cache == removal.cache {
			mplex.caches = append(mplex.caches[:i], mplex.caches[i+1:]...)
			mplex.onCacheRemoved(cache, removal.err)
			break
		}
	}
}

func (mplex *cacheMultiplexer) Fetch(target string, key string, files []string) (bool, []string, int, error) {
	// Make a shallow copy of the caches, since storeUntil can call removeCache
	mplex.mu.RLock()
	caches := make([]Cache, len(mplex.caches))
	copy(caches, mplex.caches)
	mplex.mu.RUnlock()

	// Retrieve from caches sequentially; if we did them simultaneously we could
	// easily write the same file from two goroutines at once.
	for i, cache := range caches {
		ok, actualFiles, duration, err := cache.Fetch(target, key, files)
		if err != nil {
			cd := &util.CacheDisabledError{}
			if errors.As(err, &cd) {
				mplex.removeCache(&cacheRemoval{
					cache: cache,
					err:   cd,
				})
			}
			// We're ignoring the error in the else case, since with this cache
			// abstraction, we want to check lower priority caches rather than fail
			// the operation. Future work that plumbs UI / Logging into the cache system
			// should probably log this at least.
		}
		if ok {
			// Store this into other caches. We can ignore errors here because we know
			// we have previously successfully stored in a higher-priority cache, and so the overall
			// result is a success at fetching. Storing in lower-priority caches is an optimization.
			_ = mplex.storeUntil(target, key, duration, actualFiles, i)
			return ok, actualFiles, duration, err
		}
	}
	return false, files, 0, nil
}

func (mplex *cacheMultiplexer) Clean(target string) {
	for _, cache := range mplex.caches {
		cache.Clean(target)
	}
}

func (mplex *cacheMultiplexer) CleanAll() {
	for _, cache := range mplex.caches {
		cache.CleanAll()
	}
}

func (mplex *cacheMultiplexer) Shutdown() {
	for _, cache := range mplex.caches {
		cache.Shutdown()
	}
}
