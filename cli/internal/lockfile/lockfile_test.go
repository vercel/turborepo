package lockfile

import (
	"io"
	"reflect"
	"sort"
	"testing"

	"github.com/vercel/turbo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
)

func Test_ByKeySortIsStable(t *testing.T) {
	packagesA := []Package{
		{"/foo/1.2.3", "1.2.3", true},
		{"/baz/1.0.9", "/baz/1.0.9", true},
		{"/bar/1.2.3", "1.2.3", true},
		{"/foo/1.2.3", "/foo/1.2.3", true},
		{"/baz/1.0.9", "1.0.9", true},
	}
	packagesB := make([]Package, len(packagesA))
	copy(packagesB, packagesA)

	sort.Sort(ByKey(packagesA))
	sort.Sort(ByKey(packagesB))

	assert.DeepEqual(t, packagesA, packagesB)
}

type mockLockfile struct{}

func (m *mockLockfile) ResolvePackage(_ turbopath.AnchoredUnixPath, _ string, _ string) (Package, error) {
	panic("unimplemented")
}

func (m *mockLockfile) AllDependencies(_ string) (map[string]string, bool) {
	panic("unimplemented")
}

func (m *mockLockfile) Subgraph(_ []turbopath.AnchoredSystemPath, _ []string) (Lockfile, error) {
	panic("unimplemented")
}

func (m *mockLockfile) Encode(_ io.Writer) error {
	panic("unimplemented")
}

func (m *mockLockfile) Patches() []turbopath.AnchoredUnixPath {
	panic("unimplemented")
}

func (m *mockLockfile) GlobalChange(_ Lockfile) bool {
	panic("unimplemented")
}

var _ (Lockfile) = (*mockLockfile)(nil)

func Test_AllTransitiveClosureReturnsEmptySets(t *testing.T) {
	closures, err := AllTransitiveClosures(map[turbopath.AnchoredUnixPath]map[string]string{
		turbopath.AnchoredUnixPath("."): {},
		turbopath.AnchoredUnixPath("a"): {},
		turbopath.AnchoredUnixPath("b"): {},
	}, &mockLockfile{})
	assert.NilError(t, err)
	assert.Assert(t, len(closures) == 3)
	for _, closure := range closures {
		assert.Assert(t, closure != nil && !reflect.ValueOf(closure).IsNil())
		assert.Equal(t, closure.Cardinality(), 0)
	}
}
