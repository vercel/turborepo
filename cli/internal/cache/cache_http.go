package cache

import (
	"archive/tar"
	"compress/gzip"
	"fmt"
	"io"
	"io/ioutil"
	log "log"
	"net/http"
	"os"
	"path"
	"strconv"
	"time"

	"github.com/vercel/turborepo/cli/internal/analytics"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/fs"
)

type httpCache struct {
	writable       bool
	config         *config.Config
	requestLimiter limiter
	recorder       analytics.Recorder
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

func (cache *httpCache) Put(target, hash string, duration int, files []string) error {
	// if cache.writable {
	cache.requestLimiter.acquire()
	defer cache.requestLimiter.release()

	r, w := io.Pipe()
	go cache.write(w, hash, files)
	return cache.config.ApiClient.PutArtifact(hash, duration, r)
}

// write writes a series of files into the given Writer.
func (cache *httpCache) write(w io.WriteCloser, hash string, files []string) {
	defer w.Close()
	gzw := gzip.NewWriter(w)
	defer gzw.Close()
	tw := tar.NewWriter(gzw)
	defer tw.Close()
	for _, file := range files {
		// log.Printf("caching file %v", file)
		if err := cache.storeFile(tw, file); err != nil {
			log.Printf("[ERROR] Error uploading artifacts to HTTP cache: %s", err)
			// TODO(jaredpalmer): How can we cancel the request at this point?
		}
	}
}

func (cache *httpCache) storeFile(tw *tar.Writer, name string) error {
	info, err := os.Lstat(name)
	if err != nil {
		return err
	}
	target := ""
	if info.Mode()&os.ModeSymlink != 0 {
		target, _ = os.Readlink(name)
	}
	hdr, err := tar.FileInfoHeader(info, target)
	if err != nil {
		return err
	}
	hdr.Name = name
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
	f, err := os.Open(name)
	if err != nil {
		return err
	}
	defer f.Close()
	_, err = io.Copy(tw, f)
	return err
}

func (cache *httpCache) Fetch(target, key string, _unusedOutputGlobs []string) (bool, []string, int, error) {
	cache.requestLimiter.acquire()
	defer cache.requestLimiter.release()
	hit, files, duration, err := cache.retrieve(key)
	if err != nil {
		// TODO: analytics event?
		return false, files, duration, fmt.Errorf("failed to retrieve files from HTTP cache: %w", err)
	}
	cache.logFetch(hit, key, duration)
	return hit, files, duration, err
}

func (cache *httpCache) logFetch(hit bool, hash string, duration int) {
	var event string
	if hit {
		event = cacheEventHit
	} else {
		event = cacheEventMiss
	}
	payload := &CacheEvent{
		Source:   "REMOTE",
		Event:    event,
		Hash:     hash,
		Duration: duration,
	}
	cache.recorder.LogEvent(payload)
}

func (cache *httpCache) retrieve(key string) (bool, []string, int, error) {
	resp, err := cache.config.ApiClient.FetchArtifact(key, nil)
	if err != nil {
		return false, nil, 0, err
	}
	defer resp.Body.Close()
	files := []string{}
	missingLinks := []*tar.Header{}
	duration := 0
	// If present, extract the duration from the response.
	if resp.Header.Get("x-artifact-duration") != "" {
		intVar, err := strconv.Atoi(resp.Header.Get("x-artifact-duration"))
		if err != nil {
			return false, nil, 0, fmt.Errorf("invalid x-artifact-duration header: %w", err)
		}
		duration = intVar
	}
	if resp.StatusCode == http.StatusNotFound {
		return false, files, duration, nil // doesn't exist - not an error
	} else if resp.StatusCode != http.StatusOK {
		b, _ := ioutil.ReadAll(resp.Body)
		return false, files, duration, fmt.Errorf("%s", string(b))
	}
	gzr, err := gzip.NewReader(resp.Body)
	if err != nil {
		return false, files, duration, err
	}
	defer gzr.Close()
	tr := tar.NewReader(gzr)
	for {
		hdr, err := tr.Next()
		if err != nil {
			if err == io.EOF {
				for _, link := range missingLinks {
					if err := os.Symlink(link.Linkname, link.Name); err != nil {
						return false, files, duration, err
					}
				}

				return true, files, duration, nil
			}
			return false, files, duration, err
		}
		files = append(files, hdr.Name)
		switch hdr.Typeflag {
		case tar.TypeDir:
			if err := os.MkdirAll(hdr.Name, fs.DirPermissions); err != nil {
				return false, files, duration, err
			}
		case tar.TypeReg:
			if dir := path.Dir(hdr.Name); dir != "." {
				if err := os.MkdirAll(dir, fs.DirPermissions); err != nil {
					return false, files, duration, err
				}
			}
			if f, err := os.OpenFile(hdr.Name, os.O_WRONLY|os.O_TRUNC|os.O_CREATE, os.FileMode(hdr.Mode)); err != nil {
				return false, files, duration, err
			} else if _, err := io.Copy(f, tr); err != nil {
				return false, files, duration, err
			} else if err := f.Close(); err != nil {
				return false, files, duration, err
			}
		case tar.TypeSymlink:
			if dir := path.Dir(hdr.Name); dir != "." {
				if err := os.MkdirAll(dir, fs.DirPermissions); err != nil {
					return false, files, duration, err
				}
			}
			if _, err := os.Lstat(hdr.Name); err == nil {
				if err := os.Remove(hdr.Name); err != nil {
					return false, files, duration, err
				}
			} else if os.IsNotExist(err) {
				missingLinks = append(missingLinks, hdr)
				continue
			}

			if err := os.Symlink(hdr.Linkname, hdr.Name); err != nil {
				return false, files, duration, err
			}
		default:
			log.Printf("Unhandled file type %d for %s", hdr.Typeflag, hdr.Name)
		}
	}
}

func (cache *httpCache) Clean(target string) {
	// Not possible; this implementation can only clean for a hash.
}

func (cache *httpCache) CleanAll() {
	// Also not possible.
}

func (cache *httpCache) Shutdown() {}

func newHTTPCache(config *config.Config, recorder analytics.Recorder) *httpCache {
	return &httpCache{
		writable:       true,
		config:         config,
		requestLimiter: make(limiter, 20),
		recorder:       recorder,
	}
}
