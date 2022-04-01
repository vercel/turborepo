package util

import (
	"fmt"
	"github.com/stretchr/testify/assert"
	"testing"
)

func Test_IsBerry(t *testing.T) {
	cases := []struct {
		Input    string
		Expected bool
	}{
		{
			"1.0.0",
			false,
		},
		{
			"2.0.0",
			true,
		},
		{
			"3.0.0",
			true,
		},
	}

	for i, tc := range cases {
		t.Run(fmt.Sprintf("%d) '%s' should return '%t'", i, tc.Input, tc.Expected), func(t *testing.T) {
			if result, err := IsBerry(".", tc.Input, true); err != nil {
				t.Fatalf("invalid input: %#v", err)
			} else {
				assert.EqualValues(t, tc.Expected, result)
			}
		})
	}
}
