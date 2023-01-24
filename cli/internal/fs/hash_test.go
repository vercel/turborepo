package fs

import (
	"testing"

	"gotest.tools/v3/assert"
)

const _numOfRuns = 20

func Test_HashObjectStability(t *testing.T) {
	type TestCase struct {
		name string
		obj  interface{}
	}
	type complexStruct struct {
		nested TaskOutputs
		foo    string
		bar    []string
	}

	testCases := []TestCase{
		{
			name: "task object",
			obj: TaskOutputs{
				Inclusions: []string{"foo", "bar"},
				Exclusions: []string{"baz"},
			},
		},
		{
			name: "complex struct",
			obj: complexStruct{
				nested: TaskOutputs{
					Exclusions: []string{"bar", "baz"},
					Inclusions: []string{"foo"},
				},
				foo: "a",
				bar: []string{"b", "c"},
			},
		},
	}

	for _, tc := range testCases {
		expectedHash, err := HashObject(tc.obj)
		assert.NilError(t, err, tc.name)

		for n := 0; n < _numOfRuns; n++ {
			hash, err := HashObject(tc.obj)
			assert.NilError(t, err, tc.name)
			assert.Equal(t, expectedHash, hash, tc.name)
		}
	}
}
