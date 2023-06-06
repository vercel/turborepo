package fs

import (
	"testing"

	"github.com/vercel/turbo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
)

func Test_RecursiveCopyCopiesFiles(t *testing.T) {
	base := turbopath.AbsoluteSystemPath(t.TempDir())
	src := base.UntypedJoin("src")
	err := src.Mkdir(0775)
	assert.NilError(t, err, "mkdir")
	err = RecursiveCopy(src, base.UntypedJoin("dst"))
	assert.NilError(t, err, "recursive copy")
}
