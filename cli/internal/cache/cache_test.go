package cache

import (
	"os"
	"reflect"
	"sync/atomic"
	"testing"

	"github.com/vercel/turborepo/cli/internal/analytics"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/turbopath"
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

type nullRecorder struct{}

func (nullRecorder) LogEvent(analytics.EventPayload) {}

func TestNew(t *testing.T) {
	// Test will bomb if this fails, no need to specially handle the error
	cwd, _ := os.Getwd()
	type args struct {
		opts           Opts
		config         *config.Config
		recorder       analytics.Recorder
		onCacheRemoved OnCacheRemoved
		client         client
	}
	tests := []struct {
		name    string
		args    args
		want    Cache
		wantErr bool
	}{
		{
			name: "With no caches configured, new returns a noopCache and an error",
			args: args{
				opts: Opts{
					SkipFilesystem: true,
					SkipRemote:     true,
				},
				config:         &config.Config{},
				recorder:       &nullRecorder{},
				onCacheRemoved: func(Cache, error) {},
			},
			want:    &noopCache{},
			wantErr: true,
		},
		{
			name: "With just httpCache configured, new returns an httpCache and a noopCache",
			args: args{
				opts: Opts{
					SkipFilesystem: true,
					SkipRemote:     false,
					RemoteCacheOpts: fs.RemoteCacheOptions{
						Signature: true,
					},
				},
				config:         &config.Config{},
				recorder:       &nullRecorder{},
				onCacheRemoved: func(Cache, error) {},
			},
			want: &cacheMultiplexer{
				caches: []Cache{&httpCache{}, &noopCache{}},
			},
			wantErr: false,
		},
		{
			name: "With just fsCache configured, new returns only an fsCache",
			args: args{
				opts: Opts{
					Dir:            turbopath.AbsolutePath(cwd),
					SkipFilesystem: false,
					SkipRemote:     true,
				},
				config:         &config.Config{},
				recorder:       &nullRecorder{},
				onCacheRemoved: func(Cache, error) {},
			},
			want:    &fsCache{},
			wantErr: false,
		},
		{
			name: "With both configured, new returns an fsCache and httpCache",
			args: args{
				opts: Opts{
					Dir:            turbopath.AbsolutePath(cwd),
					SkipFilesystem: false,
					SkipRemote:     false,
					RemoteCacheOpts: fs.RemoteCacheOptions{
						Signature: true,
					},
				},
				config:         &config.Config{},
				recorder:       &nullRecorder{},
				onCacheRemoved: func(Cache, error) {},
			},
			want: &cacheMultiplexer{
				caches: []Cache{&fsCache{}, &httpCache{}},
			},
			wantErr: false,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := New(tt.args.opts, tt.args.config, tt.args.client, tt.args.recorder, tt.args.onCacheRemoved)
			if (err != nil) != tt.wantErr {
				t.Errorf("New() error = %v, wantErr %v", err, tt.wantErr)
				return
			}
			switch multiplexer := got.(type) {
			case *cacheMultiplexer:
				want := tt.want.(*cacheMultiplexer)
				for i := range multiplexer.caches {
					if reflect.TypeOf(multiplexer.caches[i]) != reflect.TypeOf(want.caches[i]) {
						t.Errorf("New() = %v, want %v", reflect.TypeOf(multiplexer.caches[i]), reflect.TypeOf(want.caches[i]))
					}
				}
			case *fsCache:
				if reflect.TypeOf(got) != reflect.TypeOf(tt.want) {
					t.Errorf("New() = %v, want %v", reflect.TypeOf(got), reflect.TypeOf(tt.want))
				}
			case *noopCache:
				if reflect.TypeOf(got) != reflect.TypeOf(tt.want) {
					t.Errorf("New() = %v, want %v", reflect.TypeOf(got), reflect.TypeOf(tt.want))
				}
			}
		})
	}
}
