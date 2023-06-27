package fs

import (
	"testing"

	"gotest.tools/v3/assert"
)

const _numOfRuns = 20

func Test_HashObjectStability(t *testing.T) {
	type TestCase struct {
		name         string
		expectedHash string
		obj          interface{}
	}
	type complexStruct struct {
		Nested TaskOutputs
		Foo    string
		Bar    []string
	}

	testCases := []TestCase{
		{
			name:         "task object",
			expectedHash: "6ea4cef295ea772c",
			obj: TaskOutputs{
				Inclusions: []string{"foo", "bar"},
				Exclusions: []string{"baz"},
			},
		},
		{
			name:         "complex struct",
			expectedHash: "d55de0d0e0944858",
			obj: complexStruct{
				Nested: TaskOutputs{
					Exclusions: []string{"bar", "baz"},
					Inclusions: []string{"foo"},
				},
				Foo: "a",
				Bar: []string{"b", "c"},
			},
		},
	}

	for _, tc := range testCases {
		t.Run(tc.name, func(t *testing.T) {
			for n := 0; n < _numOfRuns; n++ {
				hash, err := HashObject(tc.obj)
				assert.NilError(t, err, tc.name)
				assert.Equal(t, tc.expectedHash, hash, tc.name)
			}
		})
	}
}
