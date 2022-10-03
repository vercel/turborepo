package colorcache

import (
	"sync"

	"github.com/vercel/turborepo/cli/internal/util"

	"github.com/fatih/color"
)

type colorFn = func(format string, a ...interface{}) string

func getTerminalPackageColors() []colorFn {
	return []colorFn{color.CyanString, color.MagentaString, color.GreenString, color.YellowString, color.BlueString}
}

type ColorCache struct {
	mu         sync.Mutex
	index      int
	TermColors []colorFn
	Cache      map[interface{}]colorFn
}

// New creates an instance of ColorCache with helpers for adding colors to task outputs
func New() *ColorCache {
	return &ColorCache{
		TermColors: getTerminalPackageColors(),
		index:      0,
		Cache:      make(map[interface{}]colorFn),
	}
}

// colorForKey returns a color function for a given package name
func (c *ColorCache) colorForKey(key string) colorFn {
	c.mu.Lock()
	defer c.mu.Unlock()
	colorFn, ok := c.Cache[key]
	if ok {
		return colorFn
	}
	c.index++
	colorFn = c.TermColors[util.PositiveMod(c.index, 5)] // 5 possible colors
	c.Cache[key] = colorFn
	return colorFn
}

// PrefixWithColor returns a string consisting of the provided prefix in a consistent
// color based on the cacheKey
func (c *ColorCache) PrefixWithColor(cacheKey string, prefix string) string {
	colorFn := c.colorForKey(cacheKey)
	return colorFn("%s: ", prefix)
}
