package sub

import (
	"strings"

	"golang.org/x/old/thing"
)

// Sub is in the module cache.
func Sub() string { return strings.TrimSpace(thing.X) }
