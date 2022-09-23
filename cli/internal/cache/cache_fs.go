// Adapted from https://github.com/thought-machine/please
// Copyright Thought Machine, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
package cache

import (
	"encoding/json"
	"fmt"
	"io/ioutil"
	"runtime"

	"github.com/vercel/turborepo/cli/internal/analytics"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/turbopath"
	"golang.org/x/sync/errgroup"
)

// fsCache is a local filesystem cache
type fsCache struct {
	cacheDirectory turbopath.AbsoluteSystemPath
	recorder       analytics.Recorder
	repoRoot       turbopath.AbsoluteSystemPath
}

// newFsCache creates a new filesystem cache
func newFsCache(opts Opts, recorder analytics.Recorder, repoRoot turbopath.AbsoluteSystemPath) (*fsCache, error) {
	cacheDir := opts.ResolveCacheDir(repoRoot)
	if err := cacheDir.MkdirAll(); err != nil {
		return nil, err
	}
	return &fsCache{
		cacheDirectory: cacheDir,
		recorder:       recorder,
		repoRoot:       repoRoot,
	}, nil
}

// Fetch returns true if items are cached. It moves them into position as a side effect.
func (f *fsCache) Fetch(anchor turbopath.AbsoluteSystemPath, hash string, _unusedOutputGlobs []string) (bool, []turbopath.AnchoredSystemPath, int, error) {
	cachedFolder := f.cacheDirectory.UntypedJoin(hash)

	// If it's not in the cache bail now
	if !cachedFolder.DirExists() {
		f.logFetch(false, hash, 0)
		return false, nil, 0, nil
	}

	// Otherwise, copy it into position
	err := fs.RecursiveCopy(cachedFolder.ToStringDuringMigration(), anchor.ToStringDuringMigration())
	if err != nil {
		// TODO: what event to log here?
		return false, nil, 0, fmt.Errorf("error moving artifact from cache into %v: %w", anchor, err)
	}

	meta, err := ReadCacheMetaFile(f.cacheDirectory.UntypedJoin(hash + "-meta.json"))
	if err != nil {
		return false, nil, 0, fmt.Errorf("error reading cache metadata: %w", err)
	}
	f.logFetch(true, hash, meta.Duration)
	return true, nil, meta.Duration, nil
}

func (f *fsCache) Exists(hash string) (ItemStatus, error) {
	cachedFolder := f.cacheDirectory.UntypedJoin(hash)

	if cachedFolder.Exists() {
		return ItemStatus{Local: false}, nil
	}

	return ItemStatus{Local: true}, nil
}

func (f *fsCache) logFetch(hit bool, hash string, duration int) {
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
	}
	f.recorder.LogEvent(payload)
}

func (f *fsCache) Put(anchor turbopath.AbsoluteSystemPath, hash string, duration int, files []turbopath.AnchoredSystemPath) error {
	g := new(errgroup.Group)

	numDigesters := runtime.NumCPU()
	fileQueue := make(chan turbopath.AnchoredSystemPath, numDigesters)

	for i := 0; i < numDigesters; i++ {
		g.Go(func() error {
			for file := range fileQueue {
				statedFile := fs.LstatCachedFile{Path: file.RestoreAnchor(anchor)}
				fromType, err := statedFile.GetType()
				if err != nil {
					return fmt.Errorf("error stat'ing cache source %v: %v", file, err)
				}
				if !fromType.IsDir() {
					if err := f.cacheDirectory.UntypedJoin(hash, file.ToStringDuringMigration()).EnsureDir(); err != nil {
						return fmt.Errorf("error ensuring directory file from cache: %w", err)
					}

					if err := fs.CopyFile(&statedFile, f.cacheDirectory.UntypedJoin(hash, file.ToStringDuringMigration()).ToStringDuringMigration()); err != nil {
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

	WriteCacheMetaFile(f.cacheDirectory.UntypedJoin(hash+"-meta.json").ToString(), &CacheMetadata{
		Duration: duration,
		Hash:     hash,
	})

	return nil
}

func (f *fsCache) Clean(anchor turbopath.AbsoluteSystemPath) {
	fmt.Println("Not implemented yet")
}

func (f *fsCache) CleanAll() {
	fmt.Println("Not implemented yet")
}

func (cache *fsCache) Shutdown() {}

// CacheMetadata stores duration and hash information for a cache entry so that aggregate Time Saved calculations
// can be made from artifacts from various caches
type CacheMetadata struct {
	Hash     string `json:"hash"`
	Duration int    `json:"duration"`
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
func ReadCacheMetaFile(path turbopath.AbsoluteSystemPath) (*CacheMetadata, error) {
	jsonBytes, readFileErr := path.ReadFile()
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
