package cache

import (
	"bufio"
	"encoding/json"
	"fmt"
	"io/ioutil"
	"os"
	"path/filepath"
	"runtime"
	"time"

	"github.com/vercel/turborepo/cli/internal/analytics"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/fs"
	"golang.org/x/sync/errgroup"
)

// fsCache is a local filesystem cache
type fsCache struct {
	cacheDirectory string
	recorder       analytics.Recorder
}

// newFsCache creates a new filesystem cache
func newFsCache(config *config.Config, recorder analytics.Recorder) Cache {
	return &fsCache{cacheDirectory: config.Cache.Dir, recorder: recorder}
}

// Fetch returns true if items are cached. It moves them into position as a side effect.
func (f *fsCache) Fetch(target, hash string, _unusedOutputGlobs []string) (bool, []string, time.Time, int, error) {
	cachedFolder := filepath.Join(f.cacheDirectory, hash)

	// If it's not in the cache bail now
	if !fs.PathExists(cachedFolder) {
		f.logFetch(false, hash, notime, 0)
		return false, nil, notime, 0, nil
	}

	// Otherwise, copy it into position
	err := fs.RecursiveCopyOrLinkFile(cachedFolder, target, fs.DirPermissions, true, true)
	if err != nil {
		// TODO: what event to log here?
		return false, nil, notime, 0, fmt.Errorf("error moving artifact from cache into %v: %w", target, err)
	}

	meta, err := ReadCacheMetaFile(filepath.Join(f.cacheDirectory, hash+"-meta.json"))
	if err != nil {
		return false, nil, notime, 0, fmt.Errorf("error reading cache metadata: %w", err)
	}
	f.logFetch(true, hash, meta.Start, meta.Duration)
	return true, nil, meta.Start, meta.Duration, nil
}

func (f *fsCache) logFetch(hit bool, hash string, start time.Time, duration int) {
	var event string
	if hit {
		event = cacheEventHit
	} else {
		event = cacheEventMiss
	}
	payload := &CacheEvent{
		Source:   "LOCAL",
		Event:    event,
		Hash:     hash,
		Duration: duration,
		Start:    start,
	}
	f.recorder.LogEvent(payload)
}

func (f *fsCache) Put(target, hash string, start time.Time, duration int, files []string) error {
	g := new(errgroup.Group)

	numDigesters := runtime.NumCPU()
	fileQueue := make(chan string, numDigesters)

	for i := 0; i < numDigesters; i++ {
		g.Go(func() error {
			for file := range fileQueue {
				if !fs.IsDirectory(file) {
					if err := fs.EnsureDir(filepath.Join(f.cacheDirectory, hash, file)); err != nil {
						return fmt.Errorf("error ensuring directory file from cache: %w", err)
					}

					if err := fs.CopyOrLinkFile(file, filepath.Join(f.cacheDirectory, hash, file), fs.DirPermissions, fs.DirPermissions, true, true); err != nil {
						return fmt.Errorf("error copying file from cache: %w", err)
					}
				}
			}
			return nil
		})
	}

	for _, file := range files {
		fileQueue <- file
	}
	close(fileQueue)

	if err := g.Wait(); err != nil {
		return err
	}

	WriteCacheMetaFile(filepath.Join(f.cacheDirectory, hash+"-meta.json"), &CacheMetadata{
		Duration: duration,
		Hash:     hash,
		Start:    start,
	})

	return nil
}

func (f *fsCache) Clean(target string) {
	fmt.Println("Not implemented yet")
}

func (f *fsCache) CleanAll() {
	fmt.Println("Not implemented yet")
}

func (cache *fsCache) Shutdown() {}

// CacheMetadata stores duration and hash information for a cache entry so that aggregate Time Saved calculations
// can be made from artifacts from various caches
type CacheMetadata struct {
	Hash     string    `json:"hash"`
	Duration int       `json:"duration"`
	Start    time.Time `json:"start"`
}

// WriteCacheMetaFile writes cache metadata file at a path
func WriteCacheMetaFile(path string, config *CacheMetadata) error {
	jsonBytes, marshalErr := json.Marshal(config)
	if marshalErr != nil {
		return marshalErr
	}
	writeFilErr := ioutil.WriteFile(path, jsonBytes, 0644)
	if writeFilErr != nil {
		return writeFilErr
	}
	return nil
}

// ReadCacheMetaFile reads cache metadata file at a path
func ReadCacheMetaFile(path string) (*CacheMetadata, error) {
	jsonBytes, readFileErr := ioutil.ReadFile(path)
	if readFileErr != nil {
		return nil, readFileErr
	}
	var config CacheMetadata
	marshalErr := json.Unmarshal(jsonBytes, &config)
	if marshalErr != nil {
		return nil, marshalErr
	}
	return &config, nil
}

// AppendHashesFile adds a hash to a file at path
// Note: naively assuming locks are not needed
func AppendHashesFile(path string, hash string) error {
	file, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)
	if err != nil {
		return err
	}

	defer file.Close()

	if _, err = file.WriteString(hash + "\n"); err != nil {
		return err
	}

	return nil
}

// ReadHashesFile reads hashes stored line by line from a file at path
func ReadHashesFile(path string) ([]string, error) {
	file, err := os.Open(path)
	if err != nil {
		return nil, err
	}

	defer file.Close()

	var hashes []string
	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		hashes = append(hashes, scanner.Text())
	}
	return hashes, scanner.Err()
}
