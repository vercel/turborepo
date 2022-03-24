package util

import (
	"fmt"
	"github.com/stretchr/testify/assert"
	"testing"
)

func TestParseConcurrency(t *testing.T) {
	cases := []struct {
		Input    string
		Expected int
	}{
		{
			"12",
			12,
		},
		{
			"200%",
			20,
		},
		{
			"100%",
			10,
		},
		{
			"50%",
			5,
		},
		{
			"25%",
			2,
		},
		{
			"1%",
			1,
		},
	}

	// mock runtime.NumCPU() to 10
	runtimeNumCPU = func() int {
		return 10
	}

	for i, tc := range cases {
		t.Run(fmt.Sprintf("%d) '%s' should be parsed at '%d'", i, tc.Input, tc.Expected), func(t *testing.T) {
			if result, err := ParseConcurrency(tc.Input); err != nil {
				t.Fatalf("invalid parse: %#v", err)
			} else {
				assert.EqualValues(t, tc.Expected, result)
			}
		})
	}

	t.Run("throw on invalid string input", func(t *testing.T) {
		_, err := ParseConcurrency("asdf")
		assert.Error(t, err)
	})

	t.Run("throw on invalid number input", func(t *testing.T) {
		_, err := ParseConcurrency("-1")
		assert.Error(t, err)
	})

	t.Run("throw on invalid percent input - negative", func(t *testing.T) {
		_, err := ParseConcurrency("-1%")
		assert.Error(t, err)
	})
}
