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

	"github.com/spf13/pflag"
	"github.com/vercel/turborepo/cli/internal/analytics"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/turbopath"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"
	"golang.org/x/sync/errgroup"
)

// Cache is abstracted way to cache/fetch previously run tasks
type Cache interface {
	// Fetch returns true if there is a cache it. It is expected to move files
	// into their correct position as a side effect
	Fetch(target string, hash string, files []string) (bool, []string, int, error)
	Exists(hash string) (CacheState, error)
	// Put caches files for a given hash
	Put(target string, hash string, duration int, files []string) error
	Clean(target string)
	CleanAll()
	Shutdown()
}

// CacheState holds whether artifacts exists for a given hash on local
// and/or remote caching server
type CacheState struct {
	Local  bool `json:"local"`
	Remote bool `json:"remote"`
}

const cacheEventHit = "HIT"
const cacheEventMiss = "MISS"

type CacheEvent struct {
	Source   string `mapstructure:"source"`
	Event    string `mapstructure:"event"`
	Hash     string `mapstructure:"hash"`
	Duration int    `mapstructure:"duration"`
}

// DefaultLocation returns the default filesystem cache location, given a repo root
func DefaultLocation(repoRoot turbopath.AbsolutePath) turbopath.AbsolutePath {
	return repoRoot.Join("node_modules", ".cache", "turbo")
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

// ResolveCacheDir calculates the location turbo should use to cache artifacts,
// based on the options supplied by the user.
func (o *Opts) ResolveCacheDir(repoRoot turbopath.AbsolutePath) turbopath.AbsolutePath {
	if o.OverrideDir != "" {
		return fs.ResolveUnknownPath(repoRoot, o.OverrideDir)
	}
	return DefaultLocation(repoRoot)
}

var _remoteOnlyHelp = `Ignore the local filesystem cache for all tasks. Only
allow reading and caching artifacts using the remote cache.`

// AddFlags adds cache-related flags to the given FlagSet
func AddFlags(opts *Opts, flags *pflag.FlagSet) {
	// skipping remote caching not currently a flag
	flags.BoolVar(&opts.SkipFilesystem, "remote-only", false, _remoteOnlyHelp)
	flags.StringVar(&opts.OverrideDir, "cache-dir", "", "Override the filesystem cache directory.")
	flags.IntVar(&opts.Workers, "cache-workers", 10, "Set the number of concurrent cache operations")
}

// New creates a new cache
func New(opts Opts, repoRoot turbopath.AbsolutePath, client client, recorder analytics.Recorder, onCacheRemoved OnCacheRemoved) (Cache, error) {
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
func newSyncCache(opts Opts, repoRoot turbopath.AbsolutePath, client client, recorder analytics.Recorder, onCacheRemoved OnCacheRemoved) (Cache, error) {
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
		fmt.Println(ui.Dim("• Remote computation caching enabled"))
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
func (mplex *cacheMultiplexer) storeUntil(target string, key string, duration int, files []string, stopAt int) error {
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
			err := c.Put(target, key, duration, files)
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

func (mplex *cacheMultiplexer) Exists(target string) (CacheState, error) {
	syncCacheState := CacheState{}
	for _, cache := range mplex.caches {
		cacheState, err := cache.Exists(target)
		if err != nil {
			return syncCacheState, err
		}
		syncCacheState.Local = syncCacheState.Local || cacheState.Local
		syncCacheState.Remote = syncCacheState.Remote || cacheState.Remote
	}

	return syncCacheState, nil
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
