package config

import (
	"fmt"
	"os"
	"testing"

	"github.com/stretchr/testify/assert"
)

func TestSelectCwd(t *testing.T) {
	defaultCwd, err := os.Getwd()
	if err != nil {
		t.Errorf("failed to get cwd: %v", err)
	}

	cases := []struct {
		Name      string
		InputArgs []string
		Expected  string
	}{
		{
			Name:      "default",
			InputArgs: []string{"foo"},
			Expected:  defaultCwd,
		},
		{
			Name:      "choose command-line flag cwd",
			InputArgs: []string{"foo", "--cwd=zop"},
			Expected:  "zop",
		},
		{
			Name:      "ignore other flags not cwd",
			InputArgs: []string{"foo", "--ignore-this-1", "--cwd=zop", "--ignore-this=2"},
			Expected:  "zop",
		},
		{
			Name:      "ignore args after pass through",
			InputArgs: []string{"foo", "--", "--cwd=zop"},
			Expected:  defaultCwd,
		},
	}

	for i, tc := range cases {
		t.Run(fmt.Sprintf("%d-%s", i, tc.Name), func(t *testing.T) {
			actual, err := selectCwd(tc.InputArgs)
			if err != nil {
				t.Fatalf("invalid parse: %#v", err)
			}
			assert.EqualValues(t, tc.Expected, actual)
		})
	}

}
