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
	"time"
	"turbo/internal/config"
	"turbo/internal/fs"
)

type httpCache struct {
	cwd            string
	writable       bool
	config         *config.Config
	requestLimiter limiter
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

func (cache *httpCache) Put(target, hash string, files []string) error {
	// if cache.writable {
	cache.requestLimiter.acquire()
	defer cache.requestLimiter.release()
	r, w := io.Pipe()
	go cache.write(w, hash, files)
	return cache.config.ApiClient.PutArtifact(hash, cache.config.TeamId, cache.config.ProjectId, r)
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

func (cache *httpCache) Fetch(target, key string, _unusedOutputGlobs []string) (bool, []string, error) {
	cache.requestLimiter.acquire()
	defer cache.requestLimiter.release()
	m, files, err := cache.retrieve(key)
	if err != nil {
		return false, files, fmt.Errorf("Failed to retrieve files from HTTP cache: %w", err)
	}
	return m, files, err
}

func (cache *httpCache) retrieve(key string) (bool, []string, error) {
	resp, err := cache.config.ApiClient.FetchArtifact(key, cache.config.TeamId, cache.config.ProjectId, nil)
	defer resp.Body.Close()
	files := []string{}
	if resp.StatusCode == http.StatusNotFound {
		return false, files, nil // doesn't exist - not an error
	} else if resp.StatusCode != http.StatusOK {
		b, _ := ioutil.ReadAll(resp.Body)
		return false, files, fmt.Errorf("%s", string(b))
	}
	gzr, err := gzip.NewReader(resp.Body)
	if err != nil {
		return false, files, err
	}
	defer gzr.Close()
	tr := tar.NewReader(gzr)
	for {
		hdr, err := tr.Next()
		if err != nil {
			if err == io.EOF {
				return true, files, nil
			}
			return false, files, err
		}
		files = append(files, hdr.Name)
		switch hdr.Typeflag {
		case tar.TypeDir:
			if err := os.MkdirAll(hdr.Name, fs.DirPermissions); err != nil {
				return false, files, err
			}
		case tar.TypeReg:
			if dir := path.Dir(hdr.Name); dir != "." {
				if err := os.MkdirAll(dir, fs.DirPermissions); err != nil {
					return false, files, err
				}
			}
			if f, err := os.OpenFile(hdr.Name, os.O_WRONLY|os.O_TRUNC|os.O_CREATE, os.FileMode(hdr.Mode)); err != nil {
				return false, files, err
			} else if _, err := io.Copy(f, tr); err != nil {
				return false, files, err
			} else if err := f.Close(); err != nil {
				return false, files, err
			}
		case tar.TypeSymlink:
			if err := os.Symlink(hdr.Linkname, hdr.Name); err != nil {
				return false, files, err
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

func newHTTPCache(config *config.Config) *httpCache {
	return &httpCache{
		writable:       true,
		config:         config,
		requestLimiter: make(limiter, 20),
	}
}
