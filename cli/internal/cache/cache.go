// Package cache abstracts storing and fetching previously run tasks
//
// Adapted from https://github.com/thought-machine/please
// Copyright Thought Machine, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
package cache

import (
	"errors"
	"sync"

	"github.com/vercel/turbo/cli/internal/analytics"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/util"
	"golang.org/x/sync/errgroup"
)

// Cache is abstracted way to cache/fetch previously run tasks
type Cache interface {
	// Fetch returns true if there is a cache it. It is expected to move files
	// into their correct position as a side effect
	Fetch(anchor turbopath.AbsoluteSystemPath, hash string, files []string) (ItemStatus, []turbopath.AnchoredSystemPath, error)
	Exists(hash string) ItemStatus
	// Put caches files for a given hash
	Put(anchor turbopath.AbsoluteSystemPath, hash string, duration int, files []turbopath.AnchoredSystemPath) error
	Clean(anchor turbopath.AbsoluteSystemPath)
	CleanAll()
	Shutdown()
}

// ItemStatus holds whether artifacts exists for a given hash on local
// and/or remote caching server
type ItemStatus struct {
	Hit       bool
	Source    string // only relevant if Hit is true
	TimeSaved int    // will be 0 if Hit is false
}

// NewCacheMiss returns an ItemStatus with the fields set to indicate a cache miss
func NewCacheMiss() ItemStatus {
	return ItemStatus{
		Source:    CacheSourceNone,
		Hit:       false,
		TimeSaved: 0,
	}
}

// newFSTaskCacheStatus returns an ItemStatus with the fields set to indicate a local cache hit
func newFSTaskCacheStatus(hit bool, timeSaved int) ItemStatus {
	return ItemStatus{
		Source:    CacheSourceFS,
		Hit:       hit,
		TimeSaved: timeSaved,
	}
}

func newRemoteTaskCacheStatus(hit bool, timeSaved int) ItemStatus {
	return ItemStatus{
		Source:    CacheSourceRemote,
		Hit:       hit,
		TimeSaved: timeSaved,
	}
}

const (
	// CacheSourceFS is a constant to indicate local cache hit
	CacheSourceFS = "LOCAL"
	// CacheSourceRemote is a constant to indicate remote cache hit
	CacheSourceRemote = "REMOTE"
	// CacheSourceNone is an empty string because there is no source for a cache miss
	CacheSourceNone = ""
	// CacheEventHit is a constant to indicate a cache hit
	CacheEventHit = "HIT"
	// CacheEventMiss is a constant to indicate a cache miss
	CacheEventMiss = "MISS"
)

type CacheEvent struct {
	Source   string `mapstructure:"source"`
	Event    string `mapstructure:"event"`
	Hash     string `mapstructure:"hash"`
	Duration int    `mapstructure:"duration"`
}

// DefaultLocation returns the default filesystem cache location, given a repo root
func DefaultLocation(repoRoot turbopath.AbsoluteSystemPath) turbopath.AbsoluteSystemPath {
	return repoRoot.UntypedJoin("node_modules", ".cache", "turbo")
}

// OnCacheRemoved defines a callback that the cache system calls if a particular cache
// needs to be removed. In practice, this happens when Remote Caching has been disabled
// the but CLI continues to try to use it.
type OnCacheRemoved = func(cache Cache, err error)

// ErrNoCachesEnabled is returned when both the filesystem and http cache are unavailable
var ErrNoCachesEnabled = errors.New("no caches are enabled")

// Opts holds configuration options for the cache
// TODO(gsoltis): further refactor this into fs cache opts and http cache opts
type Opts struct {
	OverrideDir     string
	SkipRemote      bool
	SkipFilesystem  bool
	Workers         int
	RemoteCacheOpts fs.RemoteCacheOptions
}

// resolveCacheDir calculates the location turbo should use to cache artifacts,
// based on the options supplied by the user.
func (o *Opts) resolveCacheDir(repoRoot turbopath.AbsoluteSystemPath) turbopath.AbsoluteSystemPath {
	if o.OverrideDir != "" {
		return fs.ResolveUnknownPath(repoRoot, o.OverrideDir)
	}
	return DefaultLocation(repoRoot)
}

var _remoteOnlyHelp = `Ignore the local filesystem cache for all tasks. Only
allow reading and caching artifacts using the remote cache.`

// New creates a new cache
func New(opts Opts, repoRoot turbopath.AbsoluteSystemPath, client client, recorder analytics.Recorder, onCacheRemoved OnCacheRemoved) (Cache, error) {
	c, err := newSyncCache(opts, repoRoot, client, recorder, onCacheRemoved)
	if err != nil && !errors.Is(err, ErrNoCachesEnabled) {
		return nil, err
	}
	if opts.Workers > 0 {
		return newAsyncCache(c, opts), err
	}
	return c, err
}

// newSyncCache can return an error with a usable noopCache.
func newSyncCache(opts Opts, repoRoot turbopath.AbsoluteSystemPath, client client, recorder analytics.Recorder, onCacheRemoved OnCacheRemoved) (Cache, error) {
	// Check to see if the user has turned off particular cache implementations.
	useFsCache := !opts.SkipFilesystem
	useHTTPCache := !opts.SkipRemote

	// Since the above two flags are not mutually exclusive it is possible to configure
	// yourself out of having a cache. We should tell you about it but we shouldn't fail
	// your build for that reason.
	//
	// Further, since the httpCache can be removed at runtime, we need to insert a noopCache
	// as a backup if you are configured to have *just* an httpCache.
	//
	// This is reduced from (!useFsCache && !useHTTPCache) || (!useFsCache & useHTTPCache)
	useNoopCache := !useFsCache

	// Build up an array of cache implementations, we can only ever have 1 or 2.
	cacheImplementations := make([]Cache, 0, 2)

	if useFsCache {
		implementation, err := newFsCache(opts, recorder, repoRoot)
		if err != nil {
			return nil, err
		}
		cacheImplementations = append(cacheImplementations, implementation)
	}

	if useHTTPCache {
		implementation := newHTTPCache(opts, client, recorder, repoRoot)
		cacheImplementations = append(cacheImplementations, implementation)
	}

	if useNoopCache {
		implementation := newNoopCache()
		cacheImplementations = append(cacheImplementations, implementation)
	}

	// Precisely two cache implementations:
	// fsCache and httpCache OR httpCache and noopCache
	useMultiplexer := len(cacheImplementations) > 1
	if useMultiplexer {
		// We have early-returned any possible errors for this scenario.
		return &cacheMultiplexer{
			onCacheRemoved: onCacheRemoved,
			opts:           opts,
			caches:         cacheImplementations,
		}, nil
	}

	// Precisely one cache implementation: fsCache OR noopCache
	implementation := cacheImplementations[0]
	_, isNoopCache := implementation.(*noopCache)

	// We want to let the user know something is wonky, but we don't want
	// to trigger their build to fail.
	if isNoopCache {
		return implementation, ErrNoCachesEnabled
	}
	return implementation, nil
}

// A cacheMultiplexer multiplexes several caches into one.
// Used when we have several active (eg. http, dir).
type cacheMultiplexer struct {
	caches         []Cache
	opts           Opts
	mu             sync.RWMutex
	onCacheRemoved OnCacheRemoved
}

func (mplex *cacheMultiplexer) Put(anchor turbopath.AbsoluteSystemPath, key string, duration int, files []turbopath.AnchoredSystemPath) error {
	return mplex.storeUntil(anchor, key, duration, files, len(mplex.caches))
}

type cacheRemoval struct {
	cache Cache
	err   *util.CacheDisabledError
}

// storeUntil stores artifacts into higher priority caches than the given one.
// Used after artifact retrieval to ensure we have them in eg. the directory cache after
// downloading from the RPC cache.
func (mplex *cacheMultiplexer) storeUntil(anchor turbopath.AbsoluteSystemPath, key string, duration int, files []turbopath.AnchoredSystemPath, stopAt int) error {
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
			err := c.Put(anchor, key, duration, files)
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

func (mplex *cacheMultiplexer) Fetch(anchor turbopath.AbsoluteSystemPath, key string, files []string) (ItemStatus, []turbopath.AnchoredSystemPath, error) {
	// Make a shallow copy of the caches, since storeUntil can call removeCache
	mplex.mu.RLock()
	caches := make([]Cache, len(mplex.caches))
	copy(caches, mplex.caches)
	mplex.mu.RUnlock()

	// Retrieve from caches sequentially; if we did them simultaneously we could
	// easily write the same file from two goroutines at once.
	for i, cache := range caches {
		itemStatus, actualFiles, err := cache.Fetch(anchor, key, files)
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

		if itemStatus.Hit {
			// Store this into other caches. We can ignore errors here because we know
			// we have previously successfully stored in a higher-priority cache, and so the overall
			// result is a success at fetching. Storing in lower-priority caches is an optimization.
			_ = mplex.storeUntil(anchor, key, itemStatus.TimeSaved, actualFiles, i)

			// Return this cache, and exit the for loop, since we don't need to keep looking.
			return itemStatus, actualFiles, nil
		}
	}

	return NewCacheMiss(), nil, nil
}

// Exists check each cache sequentially and return the first one that has a cache hit
func (mplex *cacheMultiplexer) Exists(target string) ItemStatus {
	for _, cache := range mplex.caches {
		itemStatus := cache.Exists(target)
		if itemStatus.Hit {
			return itemStatus
		}
	}

	return NewCacheMiss()
}

func (mplex *cacheMultiplexer) Clean(anchor turbopath.AbsoluteSystemPath) {
	for _, cache := range mplex.caches {
		cache.Clean(anchor)
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
