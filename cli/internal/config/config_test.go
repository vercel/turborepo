package config

import (
	"fmt"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/vercel/turborepo/cli/internal/fs"
)

func TestSelectCwd(t *testing.T) {
	defaultCwd, err := fs.GetCwd()
	if err != nil {
		t.Errorf("failed to get cwd: %v", err)
	}

	cases := []struct {
		Name      string
		InputArgs []string
		Expected  fs.AbsolutePath
	}{
		{
			Name:      "default",
			InputArgs: []string{"foo"},
			Expected:  defaultCwd,
		},
		{
			Name:      "choose command-line flag cwd",
			InputArgs: []string{"foo", "--cwd=zop"},
			Expected:  defaultCwd.Join("zop"),
		},
		{
			Name:      "ignore other flags not cwd",
			InputArgs: []string{"foo", "--ignore-this-1", "--cwd=zop", "--ignore-this=2"},
			Expected:  defaultCwd.Join("zop"),
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
