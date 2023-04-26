package lockfile

import (
	"testing"

	"github.com/vercel/turbo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
)

func Test_BerryPatches(t *testing.T) {
	contents := getRustFixture(t, "berry.lock")
	lf, err := DecodeBerryLockfile(contents, nil)
	assert.NilError(t, err)
	patches := lf.Patches()
	assert.DeepEqual(t, patches, []turbopath.AnchoredUnixPath{".yarn/patches/lodash-npm-4.17.21-6382451519.patch"})
}

func Test_EmptyBerryPatches(t *testing.T) {
	contents := getRustFixture(t, "minimal-berry.lock")
	lf, err := DecodeBerryLockfile(contents, nil)
	assert.NilError(t, err)
	patches := lf.Patches()
	assert.Assert(t, patches == nil)
}
