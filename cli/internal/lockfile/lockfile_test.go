package lockfile

import (
	"sort"
	"testing"

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
