package cache

import (
	"fmt"
	"path"
	"path/filepath"
	"turbo/internal/config"
	"turbo/internal/fs"

	"golang.org/x/sync/errgroup"
)

// fsCache is a local filesystem cache
type fsCache struct {
	cacheDirectory string
}

// newFsCache creates a new filesystem cache
func newFsCache(config *config.Config) Cache {
	return &fsCache{cacheDirectory: config.Cache.Dir}
}

// Fetch returns true if items are cached. It moves them into position as a side effect.
func (f *fsCache) Fetch(target, hash string, _unusedOutputGlobs []string) (bool, []string, error) {
	cachedFolder := path.Join(f.cacheDirectory, hash)

	// If it's not in the cache bail now
	if !fs.PathExists(cachedFolder) {
		return false, nil, nil
	}

	// Otherwise, copy it into position
	err := fs.RecursiveCopyOrLinkFile(cachedFolder, target, fs.DirPermissions, true, true)
	if err != nil {
		return false, nil, fmt.Errorf("error moving cache from artifact into %v: %w", target, err)
	}
	return true, nil, nil
}

func (f *fsCache) Put(target, hash string, files []string) error {
	g := new(errgroup.Group)
	for i, file := range files {
		_, file := i, file // https://golang.org/doc/faq#closures_and_goroutines
		hash := hash
		g.Go(func() error {
			rel, err := filepath.Rel(target, file)
			if err != nil {
				return fmt.Errorf("error constructing relative path from %v to %v: %w", target, file, err)
			}
			if !fs.IsDirectory(file) {
				if err := fs.EnsureDir(path.Join(f.cacheDirectory, hash, rel)); err != nil {
					return fmt.Errorf("error ensuring directory file from cache: %w", err)
				}

				if err := fs.CopyOrLinkFile(file, path.Join(f.cacheDirectory, hash, rel), fs.DirPermissions, fs.DirPermissions, true, true); err != nil {
					return fmt.Errorf("error copying file from cache: %w", err)
				}
			}
			return nil
		})
	}

	if err := g.Wait(); err != nil {
		return err
	}

	return nil
}

func (f *fsCache) Clean(target string) {
	fmt.Println("Not implemented yet")
}

func (f *fsCache) CleanAll() {
	fmt.Println("Not implemented yet")
}

func (cache *fsCache) Shutdown() {}
