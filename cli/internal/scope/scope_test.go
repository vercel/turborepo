package scope

import (
	"fmt"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/vercel/turborepo/cli/internal/util"
)

func TestScopedPackages(t *testing.T) {
	cases := []struct {
		Name         string
		PackageNames []string
		Pattern      []string
		Expected     util.Set
	}{
		{
			"starts with @",
			[]string{"@sample/app", "sample-app", "jared"},
			[]string{"@sample/*"},
			util.Set{"@sample/app": "@sample/app"},
		},
		{
			"return an array of matches",
			[]string{"foo", "bar", "baz"},
			[]string{"f*"},
			util.Set{"foo": "foo"},
		},
		{
			"return an array of matches",
			[]string{"foo", "bar", "baz"},
			[]string{"f*", "bar"},
			util.Set{"bar": "bar", "foo": "foo"},
		},
		{
			"return matches in the order the list were defined",
			[]string{"foo", "bar", "baz"},
			[]string{"*a*", "!f*"},
			util.Set{"bar": "bar", "baz": "baz"},
		},
	}

	for i, tc := range cases {
		t.Run(fmt.Sprintf("%d-%s", i, tc.Name), func(t *testing.T) {
			actual, err := getScopedPackages(tc.PackageNames, tc.Pattern)
			if err != nil {
				t.Fatalf("invalid scope parse: %#v", err)
			}
			assert.EqualValues(t, tc.Expected, actual)
		})
	}

	t.Run(fmt.Sprintf("%d-%s", len(cases), "throws an error if no package matches the provided scope pattern"), func(t *testing.T) {
		_, err := getScopedPackages([]string{"foo", "bar"}, []string{"baz"})
		assert.Error(t, err)
	})
}
