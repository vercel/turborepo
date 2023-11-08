package lockfile

import (
	"os"
	"sort"
	"testing"

	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
)

func getFixture(t *testing.T, name string) ([]byte, error) {
	defaultCwd, err := os.Getwd()
	if err != nil {
		t.Errorf("failed to get cwd: %v", err)
	}
	cwd := turbopath.AbsoluteSystemPath(defaultCwd)
	lockfilePath := cwd.UntypedJoin("testdata", name)
	if !lockfilePath.FileExists() {
		return nil, errors.Errorf("unable to find 'testdata/%s'", name)
	}
	return os.ReadFile(lockfilePath.ToString())
}

func Test_PnpmAliasesOverlap(t *testing.T) {
	contents, err := getFixture(t, "pnpm-absolute.yaml")
	assert.NilError(t, err)

	lockfile, err := DecodePnpmLockfile(contents)
	assert.NilError(t, err)

	closures, err := AllTransitiveClosures(map[turbopath.AnchoredUnixPath]map[string]string{"packages/a": {"@scope/parent": "^1.0.0", "another": "^1.0.0", "special": "npm:Special@1.2.3"}}, lockfile)
	assert.NilError(t, err)

	closure, ok := closures[turbopath.AnchoredUnixPath("packages/a")]
	assert.Assert(t, ok)

	deps := []Package{}
	for _, v := range closure.ToSlice() {
		dep := v.(Package)
		deps = append(deps, dep)
	}
	sort.Sort(ByKey(deps))

	assert.DeepEqual(t, deps, []Package{
		{"/@scope/child/1.0.0", "1.0.0", true},
		{"/@scope/parent/1.0.0", "1.0.0", true},
		{"/Special/1.2.3", "1.2.3", true},
		{"/another/1.0.0", "1.0.0", true},
		{"/foo/1.0.0", "1.0.0", true},
	})
}
