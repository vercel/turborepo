package fs

import (
	"testing"

	"github.com/vercel/turbo/cli/internal/fs/hash"
	"gotest.tools/v3/assert"
)

const _numOfRuns = 20

func Test_HashObjectStability(t *testing.T) {
	type TestCase struct {
		name string
		obj  interface{}
	}
	type complexStruct struct {
		nested hash.TaskOutputs
		foo    string
		bar    []string
	}

	testCases := []TestCase{
		{
			name: "task object",
			obj: hash.TaskOutputs{
				Inclusions: []string{"foo", "bar"},
				Exclusions: []string{"baz"},
			},
		},
		{
			name: "complex struct",
			obj: complexStruct{
				nested: hash.TaskOutputs{
					Exclusions: []string{"bar", "baz"},
					Inclusions: []string{"foo"},
				},
				foo: "a",
				bar: []string{"b", "c"},
			},
		},
	}

	for _, tc := range testCases {
		expectedHash, err := hashObject(tc.obj)
		assert.NilError(t, err, tc.name)

		for n := 0; n < _numOfRuns; n++ {
			hash, err := hashObject(tc.obj)
			assert.NilError(t, err, tc.name)
			assert.Equal(t, expectedHash, hash, tc.name)
		}
	}
}
