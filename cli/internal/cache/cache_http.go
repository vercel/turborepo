// Adapted from https://github.com/thought-machine/please
// Copyright Thought Machine, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
package cache

import (
	"archive/tar"
	"bytes"
	"errors"
	"fmt"
	"io"
	"io/ioutil"
	log "log"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"time"

	"github.com/DataDog/zstd"

	"github.com/vercel/turbo/cli/internal/analytics"
	"github.com/vercel/turbo/cli/internal/cacheitem"
	"github.com/vercel/turbo/cli/internal/tarpatch"
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

// mtime is the time we attach for the modification time of all files.
var mtime = time.Date(2000, time.January, 1, 0, 0, 0, 0, time.UTC)

// nobody is the usual uid / gid of the 'nobody' user.
const nobody = 65534

func (cache *httpCache) Put(_ turbopath.AbsoluteSystemPath, hash string, duration int, files []turbopath.AnchoredSystemPath) error {
	// if cache.writable {
	cache.requestLimiter.acquire()
	defer cache.requestLimiter.release()

	r, w := io.Pipe()
	go cache.write(w, hash, files)

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
	return cache.client.PutArtifact(hash, artifactBody, duration, tag)
}

// write writes a series of files into the given Writer.
func (cache *httpCache) write(w io.WriteCloser, hash string, files []turbopath.AnchoredSystemPath) {
	defer w.Close()
	defer func() { _ = w.Close() }()
	zw := zstd.NewWriter(w)
	defer func() { _ = zw.Close() }()
	tw := tar.NewWriter(zw)
	defer func() { _ = tw.Close() }()
	for _, file := range files {
		// log.Printf("caching file %v", file)
		if err := cache.storeFile(tw, file); err != nil {
			log.Printf("[ERROR] Error uploading artifact %s to HTTP cache due to: %s", file, err)
			// TODO(jaredpalmer): How can we cancel the request at this point?
		}
	}
}

func (cache *httpCache) storeFile(tw *tar.Writer, repoRelativePath turbopath.AnchoredSystemPath) error {
	absoluteFilePath := repoRelativePath.RestoreAnchor(cache.repoRoot)
	info, err := absoluteFilePath.Lstat()
	if err != nil {
		return err
	}
	target := ""
	if info.Mode()&os.ModeSymlink != 0 {
		target, err = absoluteFilePath.Readlink()
		if err != nil {
			return err
		}
	}
	hdr, err := tarpatch.FileInfoHeader(repoRelativePath.ToUnixPath(), info, filepath.ToSlash(target))
	if err != nil {
		return err
	}
	// Ensure posix path for filename written in header.
	hdr.Name = repoRelativePath.ToUnixPath().ToString()
	// Zero out all timestamps.
	hdr.ModTime = mtime
	hdr.AccessTime = mtime
	hdr.ChangeTime = mtime
	// Strip user/group ids.
	hdr.Uid = nobody
	hdr.Gid = nobody
	hdr.Uname = "nobody"
	hdr.Gname = "nobody"
	if err := tw.WriteHeader(hdr); err != nil {
		return err
	} else if info.IsDir() || target != "" {
		return nil // nothing to write
	}
	f, err := absoluteFilePath.Open()
	if err != nil {
		return err
	}
	defer func() { _ = f.Close() }()
	_, err = io.Copy(tw, f)
	if errors.Is(err, tar.ErrWriteTooLong) {
		log.Printf("Error writing %v to tar file, info: %v, mode: %v, is regular: %v", repoRelativePath, info, info.Mode(), info.Mode().IsRegular())
	}
	return err
}

func (cache *httpCache) Fetch(_ turbopath.AbsoluteSystemPath, key string, _ []string) (ItemStatus, []turbopath.AnchoredSystemPath, int, error) {
	cache.requestLimiter.acquire()
	defer cache.requestLimiter.release()
	hit, files, duration, err := cache.retrieve(key)
	if err != nil {
		// TODO: analytics event?
		return ItemStatus{Remote: false}, files, duration, fmt.Errorf("failed to retrieve files from HTTP cache: %w", err)
	}
	cache.logFetch(hit, key, duration)
	return ItemStatus{Remote: hit}, files, duration, err
}

func (cache *httpCache) Exists(key string) ItemStatus {
	cache.requestLimiter.acquire()
	defer cache.requestLimiter.release()
	hit, err := cache.exists(key)
	if err != nil {
		return ItemStatus{Remote: false}
	}
	return ItemStatus{Remote: hit}
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

func (cache *httpCache) exists(hash string) (bool, error) {
	resp, err := cache.client.ArtifactExists(hash)
	if err != nil {
		return false, nil
	}

	defer func() { err = resp.Body.Close() }()

	if resp.StatusCode == http.StatusNotFound {
		return false, nil
	} else if resp.StatusCode != http.StatusOK {
		return false, fmt.Errorf("%s", strconv.Itoa(resp.StatusCode))
	}
	return true, err
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
	// If present, extract the duration from the response.
	duration := 0
	if resp.Header.Get("x-artifact-duration") != "" {
		intVar, err := strconv.Atoi(resp.Header.Get("x-artifact-duration"))
		if err != nil {
			return false, nil, 0, fmt.Errorf("invalid x-artifact-duration header: %w", err)
		}
		duration = intVar
	}
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
			teamId:  client.GetTeamID(),
			enabled: opts.RemoteCacheOpts.Signature,
		},
	}
}
