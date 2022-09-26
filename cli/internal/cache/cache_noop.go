package cache

import "github.com/vercel/turborepo/cli/internal/turbopath"

type noopCache struct{}

func newNoopCache() *noopCache {
	return &noopCache{}
}

func (c *noopCache) Put(anchor turbopath.AbsoluteSystemPath, key string, duration int, files []turbopath.AnchoredSystemPath) error {
	return nil
}
func (c *noopCache) Fetch(anchor turbopath.AbsoluteSystemPath, key string, files []string) (bool, []turbopath.AnchoredSystemPath, int, error) {
	return false, nil, 0, nil
}
func (c *noopCache) Exists(key string) (ItemStatus, error) {
	return ItemStatus{}, nil
}

func (c *noopCache) Clean(anchor turbopath.AbsoluteSystemPath) {}
func (c *noopCache) CleanAll()                                 {}
func (c *noopCache) Shutdown()                                 {}
