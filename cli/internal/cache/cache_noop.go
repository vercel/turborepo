package cache

import "github.com/vercel/turbo/cli/internal/turbopath"

type noopCache struct{}

func newNoopCache() *noopCache {
	return &noopCache{}
}

func (c *noopCache) Put(anchor turbopath.AbsoluteSystemPath, key string, duration int, files []turbopath.AnchoredSystemPath) error {
	return nil
}
func (c *noopCache) Fetch(anchor turbopath.AbsoluteSystemPath, key string, files []string) (ItemStatus, []turbopath.AnchoredSystemPath, int, error) {
	return ItemStatus{Local: false, Remote: false}, nil, 0, nil
}
func (c *noopCache) Exists(key string) ItemStatus {
	return ItemStatus{}
}

func (c *noopCache) Clean(anchor turbopath.AbsoluteSystemPath) {}
func (c *noopCache) CleanAll()                                 {}
func (c *noopCache) Shutdown()                                 {}
