package cache

import (
	"sync/atomic"
	"testing"

	"github.com/vercel/turborepo/cli/internal/util"
)

type testCache struct {
	disabledErr *util.CacheDisabledError
	entries     map[string][]string
}

func (tc *testCache) Fetch(target string, hash string, files []string) (bool, []string, int, error) {
	if tc.disabledErr != nil {
		return false, nil, 0, tc.disabledErr
	}
	foundFiles, ok := tc.entries[hash]
	if ok {
		duration := 5
		return true, foundFiles, duration, nil
	}
	return false, nil, 0, nil
}

func (tc *testCache) Put(target string, hash string, duration int, files []string) error {
	if tc.disabledErr != nil {
		return tc.disabledErr
	}
	tc.entries[hash] = files
	return nil
}

func (tc *testCache) Clean(target string) {}
func (tc *testCache) CleanAll()           {}
func (tc *testCache) Shutdown()           {}

func newEnabledCache() *testCache {
	return &testCache{
		entries: make(map[string][]string),
	}
}

func newDisabledCache() *testCache {
	return &testCache{
		disabledErr: &util.CacheDisabledError{
			Status:  util.CachingStatusDisabled,
			Message: "remote caching is disabled",
		},
	}
}

func TestPutCachingDisabled(t *testing.T) {
	disabledCache := newDisabledCache()
	caches := []Cache{
		newEnabledCache(),
		disabledCache,
		newEnabledCache(),
		newEnabledCache(),
	}
	var removeCalled uint64
	mplex := &cacheMultiplexer{
		caches: caches,
		onCacheRemoved: func(cache Cache, err error) {
			atomic.AddUint64(&removeCalled, 1)
		},
	}

	err := mplex.Put("unused-target", "some-hash", 5, []string{"a-file"})
	if err != nil {
		// don't leak the cache removal
		t.Errorf("Put got error %v, want <nil>", err)
	}

	removes := atomic.LoadUint64(&removeCalled)
	if removes != 1 {
		t.Errorf("removes count: %v, want 1", removes)
	}

	mplex.mu.RLock()
	if len(mplex.caches) != 3 {
		t.Errorf("found %v caches, expected to have 3 after one was removed", len(mplex.caches))
	}
	for _, cache := range mplex.caches {
		if cache == disabledCache {
			t.Error("found disabled cache, expected it to be removed")
		}
	}
	mplex.mu.RUnlock()

	// subsequent Fetch should still work
	hit, _, _, err := mplex.Fetch("unused-target", "some-hash", []string{"unused", "files"})
	if err != nil {
		t.Errorf("got error fetching files: %v", err)
	}
	if !hit {
		t.Error("failed to find previously stored files")
	}

	removes = atomic.LoadUint64(&removeCalled)
	if removes != 1 {
		t.Errorf("removes count: %v, want 1", removes)
	}
}

func TestFetchCachingDisabled(t *testing.T) {
	disabledCache := newDisabledCache()
	caches := []Cache{
		newEnabledCache(),
		disabledCache,
		newEnabledCache(),
		newEnabledCache(),
	}
	var removeCalled uint64
	mplex := &cacheMultiplexer{
		caches: caches,
		onCacheRemoved: func(cache Cache, err error) {
			atomic.AddUint64(&removeCalled, 1)
		},
	}

	hit, _, _, err := mplex.Fetch("unused-target", "some-hash", []string{"unused", "files"})
	if err != nil {
		// don't leak the cache removal
		t.Errorf("Fetch got error %v, want <nil>", err)
	}
	if hit {
		t.Error("hit on empty cache, expected miss")
	}

	removes := atomic.LoadUint64(&removeCalled)
	if removes != 1 {
		t.Errorf("removes count: %v, want 1", removes)
	}

	mplex.mu.RLock()
	if len(mplex.caches) != 3 {
		t.Errorf("found %v caches, expected to have 3 after one was removed", len(mplex.caches))
	}
	for _, cache := range mplex.caches {
		if cache == disabledCache {
			t.Error("found disabled cache, expected it to be removed")
		}
	}
	mplex.mu.RUnlock()
}
