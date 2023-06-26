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

func Test_BerryTransitiveClosure(t *testing.T) {
	contents := getRustFixture(t, "berry.lock")
	lf, err := DecodeBerryLockfile(contents, map[string]string{"lodash@^4.17.21": "patch:lodash@npm%3A4.17.21#./.yarn/patches/lodash-npm-4.17.21-6382451519.patch"})
	assert.NilError(t, err)
	closures, err := AllTransitiveClosures(map[turbopath.AnchoredUnixPath]map[string]string{
		turbopath.AnchoredUnixPath(""):         {},
		turbopath.AnchoredUnixPath("apps/web"): {},
		turbopath.AnchoredUnixPath("apps/docs"): {
			"lodash": "^4.17.21",
		},
	}, lf)
	assert.NilError(t, err)
	assert.Equal(t, len(closures), 3)

	lodash := Package{
		Key:     "lodash@npm:4.17.21",
		Version: "4.17.21",
		Found:   true,
	}
	assert.Assert(t, !closures[turbopath.AnchoredUnixPath("apps/web")].Contains(lodash))
	assert.Assert(t, closures[turbopath.AnchoredUnixPath("apps/docs")].Contains(lodash))
}
