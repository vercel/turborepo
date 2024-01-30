package util

import (
	"fmt"
	"testing"

	"github.com/stretchr/testify/assert"
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
		{
			"0644", // we parse in base 10
			644,
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
}

func TestInvalidPercents(t *testing.T) {
	inputs := []string{
		"asdf",
		"-1",
		"-l%",
		"infinity%",
		"-infinity%",
		"nan%",
		"0b01",
		"0o644",
		"0xFF",
	}
	for _, tc := range inputs {
		t.Run(tc, func(t *testing.T) {
			val, err := ParseConcurrency(tc)
			assert.Error(t, err, "input %v got %v", tc, val)
		})
	}
}
