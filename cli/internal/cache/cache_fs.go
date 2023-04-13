// Adapted from https://github.com/thought-machine/please
// Copyright Thought Machine, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

// Package cache implements our cache abstraction.
package cache

import (
	"encoding/json"
	"fmt"

	"github.com/vercel/turbo/cli/internal/analytics"
	"github.com/vercel/turbo/cli/internal/cacheitem"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// fsCache is a local filesystem cache
type fsCache struct {
	cacheDirectory turbopath.AbsoluteSystemPath
	recorder       analytics.Recorder
}

// newFsCache creates a new filesystem cache
func newFsCache(opts Opts, recorder analytics.Recorder, repoRoot turbopath.AbsoluteSystemPath) (*fsCache, error) {
	cacheDir := opts.resolveCacheDir(repoRoot)
	if err := cacheDir.MkdirAll(0775); err != nil {
		return nil, err
	}
	return &fsCache{
		cacheDirectory: cacheDir,
		recorder:       recorder,
	}, nil
}

// Fetch returns true if items are cached. It moves them into position as a side effect.
func (f *fsCache) Fetch(anchor turbopath.AbsoluteSystemPath, hash string, _ []string) (ItemStatus, []turbopath.AnchoredSystemPath, int, error) {
	uncompressedCachePath := f.cacheDirectory.UntypedJoin(hash + ".tar")
	compressedCachePath := f.cacheDirectory.UntypedJoin(hash + ".tar.zst")

	var actualCachePath turbopath.AbsoluteSystemPath
	if uncompressedCachePath.FileExists() {
		actualCachePath = uncompressedCachePath
	} else if compressedCachePath.FileExists() {
		actualCachePath = compressedCachePath
	} else {
		// It's not in the cache, bail now
		f.logFetch(false, hash, 0)
		return ItemStatus{Local: false}, nil, 0, nil
	}

	cacheItem, openErr := cacheitem.Open(actualCachePath)
	if openErr != nil {
		return ItemStatus{Local: false}, nil, 0, openErr
	}

	restoredFiles, restoreErr := cacheItem.Restore(anchor)
	if restoreErr != nil {
		_ = cacheItem.Close()
		return ItemStatus{Local: false}, nil, 0, restoreErr
	}

	meta, err := ReadCacheMetaFile(f.cacheDirectory.UntypedJoin(hash + "-meta.json"))
	if err != nil {
		_ = cacheItem.Close()
		return ItemStatus{Local: false}, nil, 0, fmt.Errorf("error reading cache metadata: %w", err)
	}
	f.logFetch(true, hash, meta.Duration)

	// Wait to see what happens with close.
	closeErr := cacheItem.Close()
	if closeErr != nil {
		return ItemStatus{Local: false}, restoredFiles, 0, closeErr
	}
	return ItemStatus{Local: true}, restoredFiles, meta.Duration, nil
}

func (f *fsCache) Exists(hash string) ItemStatus {
	uncompressedCachePath := f.cacheDirectory.UntypedJoin(hash + ".tar")
	compressedCachePath := f.cacheDirectory.UntypedJoin(hash + ".tar.zst")

	if compressedCachePath.FileExists() || uncompressedCachePath.FileExists() {
		return ItemStatus{Local: true}
	}

	return ItemStatus{Local: false}
}

func (f *fsCache) logFetch(hit bool, hash string, duration int) {
	var event string
	if hit {
		event = CacheEventHit
	} else {
		event = CacheEventMiss
	}
	payload := &CacheEvent{
		Source:   CacheSourceFS,
		Event:    event,
		Hash:     hash,
		Duration: duration,
	}
	f.recorder.LogEvent(payload)
}

func (f *fsCache) Put(anchor turbopath.AbsoluteSystemPath, hash string, duration int, files []turbopath.AnchoredSystemPath) error {
	cachePath := f.cacheDirectory.UntypedJoin(hash + ".tar.zst")
	cacheItem, err := cacheitem.Create(cachePath)
	if err != nil {
		return err
	}

	for _, file := range files {
		err := cacheItem.AddFile(anchor, file)
		if err != nil {
			_ = cacheItem.Close()
			return err
		}
	}

	writeErr := WriteCacheMetaFile(f.cacheDirectory.UntypedJoin(hash+"-meta.json"), &CacheMetadata{
		Duration: duration,
		Hash:     hash,
	})

	if writeErr != nil {
		_ = cacheItem.Close()
		return writeErr
	}

	return cacheItem.Close()
}

func (f *fsCache) Clean(_ turbopath.AbsoluteSystemPath) {
	fmt.Println("Not implemented yet")
}

func (f *fsCache) CleanAll() {
	fmt.Println("Not implemented yet")
}

func (f *fsCache) Shutdown() {}

// CacheMetadata stores duration and hash information for a cache entry so that aggregate Time Saved calculations
// can be made from artifacts from various caches
type CacheMetadata struct {
	Hash     string `json:"hash"`
	Duration int    `json:"duration"`
}

// WriteCacheMetaFile writes cache metadata file at a path
func WriteCacheMetaFile(path turbopath.AbsoluteSystemPath, config *CacheMetadata) error {
	jsonBytes, marshalErr := json.Marshal(config)
	if marshalErr != nil {
		return marshalErr
	}
	writeFilErr := path.WriteFile(jsonBytes, 0644)
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
