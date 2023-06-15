package cache

import "github.com/vercel/turbo/cli/internal/turbopath"

type noopCache struct{}

func newNoopCache() *noopCache {
	return &noopCache{}
}

func (c *noopCache) Put(_ turbopath.AbsoluteSystemPath, _ string, _ int, _ []turbopath.AnchoredSystemPath) error {
	return nil
}
func (c *noopCache) Fetch(_ turbopath.AbsoluteSystemPath, _ string, _ []string) (ItemStatus, []turbopath.AnchoredSystemPath, error) {
	return NewCacheMiss(), nil, nil
}

func (c *noopCache) Exists(_ string) ItemStatus {
	return NewCacheMiss()
}

func (c *noopCache) Clean(_ turbopath.AbsoluteSystemPath) {}
func (c *noopCache) CleanAll()                            {}
func (c *noopCache) Shutdown()                            {}
