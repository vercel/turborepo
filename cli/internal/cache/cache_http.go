// Adapted from https://github.com/thought-machine/please
// Copyright Thought Machine, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
package cache

import (
	"bytes"
	"errors"
	"fmt"
	"io"
	"io/ioutil"
	"net/http"
	"strconv"

	"github.com/vercel/turbo/cli/internal/analytics"
	"github.com/vercel/turbo/cli/internal/cacheitem"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

type client interface {
	PutArtifact(hash string, body []byte, duration int, tag string) error
	FetchArtifact(hash string) (*http.Response, error)
	ArtifactExists(hash string) (*http.Response, error)
	GetTeamID() string
}

type httpCache struct {
	writable       bool
	client         client
	requestLimiter limiter
	recorder       analytics.Recorder
	signerVerifier *ArtifactSignatureAuthentication
	repoRoot       turbopath.AbsoluteSystemPath
}

type limiter chan struct{}

func (l limiter) acquire() {
	l <- struct{}{}
}

func (l limiter) release() {
	<-l
}

func (cache *httpCache) Put(anchor turbopath.AbsoluteSystemPath, hash string, duration int, files []turbopath.AnchoredSystemPath) error {
	// if cache.writable {
	cache.requestLimiter.acquire()
	defer cache.requestLimiter.release()

	r, w := io.Pipe()

	cacheErrorChan := make(chan error, 1)
	go cache.write(w, anchor, files, cacheErrorChan)

	// Read the entire artifact tar into memory so we can easily compute the signature.
	// Note: retryablehttp.NewRequest reads the files into memory anyways so there's no
	// additional overhead by doing the ioutil.ReadAll here instead.
	artifactBody, err := ioutil.ReadAll(r)
	if err != nil {
		return fmt.Errorf("failed to store files in HTTP cache: %w", err)
	}
	tag := ""
	if cache.signerVerifier.isEnabled() {
		tag, err = cache.signerVerifier.generateTag(hash, artifactBody)
		if err != nil {
			return fmt.Errorf("failed to store files in HTTP cache: %w", err)
		}
	}

	cacheCreateError := <-cacheErrorChan
	if cacheCreateError != nil {
		return cacheCreateError
	}

	return cache.client.PutArtifact(hash, artifactBody, duration, tag)
}

// write writes a series of files into the given Writer.
func (cache *httpCache) write(w io.WriteCloser, anchor turbopath.AbsoluteSystemPath, files []turbopath.AnchoredSystemPath, cacheErrorChan chan error) {
	cacheItem := cacheitem.CreateWriter(w)

	for _, file := range files {
		err := cacheItem.AddFile(anchor, file)
		if err != nil {
			_ = cacheItem.Close()
			cacheErrorChan <- err
			return
		}
	}

	cacheErrorChan <- cacheItem.Close()
}

func (cache *httpCache) Fetch(_ turbopath.AbsoluteSystemPath, key string, _ []string) (ItemStatus, []turbopath.AnchoredSystemPath, error) {
	cache.requestLimiter.acquire()
	defer cache.requestLimiter.release()
	hit, files, duration, err := cache.retrieve(key)
	if err != nil {
		// TODO: analytics event?
		return newRemoteTaskCacheStatus(false, duration), files, fmt.Errorf("failed to retrieve files from HTTP cache: %w", err)
	}
	cache.logFetch(hit, key, duration)
	return newRemoteTaskCacheStatus(hit, duration), files, err
}

func (cache *httpCache) Exists(key string) ItemStatus {
	cache.requestLimiter.acquire()
	defer cache.requestLimiter.release()
	hit, timeSaved, err := cache.exists(key)
	if err != nil {
		return newRemoteTaskCacheStatus(false, 0)
	}
	return newRemoteTaskCacheStatus(hit, timeSaved)
}

func (cache *httpCache) logFetch(hit bool, hash string, duration int) {
	var event string
	if hit {
		event = CacheEventHit
	} else {
		event = CacheEventMiss
	}
	payload := &CacheEvent{
		Source:   CacheSourceRemote,
		Event:    event,
		Hash:     hash,
		Duration: duration,
	}
	cache.recorder.LogEvent(payload)
}

func (cache *httpCache) exists(hash string) (bool, int, error) {
	resp, err := cache.client.ArtifactExists(hash)
	if err != nil {
		return false, 0, nil
	}

	defer func() { err = resp.Body.Close() }()

	if resp.StatusCode == http.StatusNotFound {
		return false, 0, nil
	} else if resp.StatusCode != http.StatusOK {
		return false, 0, fmt.Errorf("%s", strconv.Itoa(resp.StatusCode))
	}

	duration := getDurationFromResponse(resp)
	return true, duration, err
}

func (cache *httpCache) retrieve(hash string) (bool, []turbopath.AnchoredSystemPath, int, error) {
	resp, err := cache.client.FetchArtifact(hash)
	if err != nil {
		return false, nil, 0, err
	}
	defer resp.Body.Close()
	if resp.StatusCode == http.StatusNotFound {
		return false, nil, 0, nil // doesn't exist - not an error
	} else if resp.StatusCode != http.StatusOK {
		b, _ := ioutil.ReadAll(resp.Body)
		return false, nil, 0, fmt.Errorf("%s", string(b))
	}

	duration := getDurationFromResponse(resp)

	var tarReader io.Reader

	defer func() { _ = resp.Body.Close() }()
	if cache.signerVerifier.isEnabled() {
		expectedTag := resp.Header.Get("x-artifact-tag")
		if expectedTag == "" {
			// If the verifier is enabled all incoming artifact downloads must have a signature
			return false, nil, 0, errors.New("artifact verification failed: Downloaded artifact is missing required x-artifact-tag header")
		}
		b, err := ioutil.ReadAll(resp.Body)
		if err != nil {
			return false, nil, 0, fmt.Errorf("artifact verification failed: %w", err)
		}
		isValid, err := cache.signerVerifier.validate(hash, b, expectedTag)
		if err != nil {
			return false, nil, 0, fmt.Errorf("artifact verification failed: %w", err)
		}
		if !isValid {
			err = fmt.Errorf("artifact verification failed: artifact tag does not match expected tag %s", expectedTag)
			return false, nil, 0, err
		}
		// The artifact has been verified and the body can be read and untarred
		tarReader = bytes.NewReader(b)
	} else {
		tarReader = resp.Body
	}
	files, err := restoreTar(cache.repoRoot, tarReader)
	if err != nil {
		return false, nil, 0, err
	}
	return true, files, duration, nil
}

// getDurationFromResponse extracts the duration from the response header
func getDurationFromResponse(resp *http.Response) int {
	duration := 0
	if resp.Header.Get("x-artifact-duration") != "" {
		// If we had an error reading the duration header, just swallow it for now.
		if intVar, err := strconv.Atoi(resp.Header.Get("x-artifact-duration")); err == nil {
			duration = intVar
		}
	}

	return duration
}

func restoreTar(root turbopath.AbsoluteSystemPath, reader io.Reader) ([]turbopath.AnchoredSystemPath, error) {
	cache := cacheitem.FromReader(reader, true)
	return cache.Restore(root)
}

func (cache *httpCache) Clean(_ turbopath.AbsoluteSystemPath) {
	// Not possible; this implementation can only clean for a hash.
}

func (cache *httpCache) CleanAll() {
	// Also not possible.
}

func (cache *httpCache) Shutdown() {}

func newHTTPCache(opts Opts, client client, recorder analytics.Recorder, repoRoot turbopath.AbsoluteSystemPath) *httpCache {
	return &httpCache{
		writable:       true,
		client:         client,
		requestLimiter: make(limiter, 20),
		recorder:       recorder,
		repoRoot:       repoRoot,
		signerVerifier: &ArtifactSignatureAuthentication{
			// TODO(Gaspar): this should use RemoteCacheOptions.TeamId once we start
			// enforcing team restrictions for repositories.
			teamID:  client.GetTeamID(),
			enabled: opts.RemoteCacheOpts.Signature,
		},
	}
}
