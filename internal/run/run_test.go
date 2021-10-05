package run

import (
	"fmt"
	"testing"
	"turbo/internal/context"
	"turbo/internal/util"

	"github.com/stretchr/testify/assert"
)

func TestParseConfig(t *testing.T) {
	cases := []struct {
		Name     string
		Args     []string
		Expected *RunOptions
	}{
		{
			"string flags",
			[]string{"foo"},
			&RunOptions{
				deps:           true,
				stream:         true,
				bail:           true,
				dotGraph:       "",
				concurrency:    10,
				ancestors:      false,
				cache:          true,
				forceExecution: false,
				profile:        "",
				cacheFolder:    "node_modules/.cache/turbo",
			},
		},
		{
			"cwd",
			[]string{"foo", "--cwd=zop"},
			&RunOptions{
				deps:           true,
				stream:         true,
				bail:           true,
				dotGraph:       "",
				concurrency:    10,
				ancestors:      false,
				cache:          true,
				forceExecution: false,
				profile:        "",
				cwd:            "zop",
				cacheFolder:    "zop/node_modules/.cache/turbo",
			},
		},
		{
			"scope",
			[]string{"foo", "--scope=foo", "--scope=blah"},
			&RunOptions{
				deps:           true,
				stream:         true,
				bail:           true,
				dotGraph:       "",
				concurrency:    10,
				ancestors:      false,
				cache:          true,
				forceExecution: false,
				profile:        "",
				scope:          []string{"foo", "blah"},
				cacheFolder:    "node_modules/.cache/turbo",
			},
		},
		{
			"concurrency",
			[]string{"foo", "--concurrency=12"},
			&RunOptions{
				deps:           true,
				stream:         true,
				bail:           true,
				dotGraph:       "",
				concurrency:    12,
				ancestors:      false,
				cache:          true,
				forceExecution: false,
				profile:        "",
				cacheFolder:    "node_modules/.cache/turbo",
			},
		},
		{
			"graph",
			[]string{"foo", "--graph=g.png"},
			&RunOptions{
				deps:           true,
				stream:         true,
				bail:           true,
				dotGraph:       "g.png",
				concurrency:    10,
				ancestors:      false,
				cache:          true,
				forceExecution: false,
				profile:        "",
				cacheFolder:    "node_modules/.cache/turbo",
			},
		},
	}

	for i, tc := range cases {
		t.Run(fmt.Sprintf("%d-%s", i, tc.Name), func(t *testing.T) {

			actual, err := parseRunArgs(tc.Args, ".")
			if err != nil {
				t.Fatalf("invalid parse: %#v", err)
			}
			assert.EqualValues(t, actual, tc.Expected)
		})
	}
}

func TestScopedPackages(t *testing.T) {
	cases := []struct {
		Name     string
		Ctx      *context.Context
		Patttern []string
		Expected util.Set
	}{
		{
			"starts with @",
			&context.Context{
				PackageNames: []string{"@sample/app", "sample-app", "jared"},
			},
			[]string{"@sample/*"},
			util.Set{"@sample/app": "@sample/app"},
		},
		{
			"return an array of matches",
			&context.Context{
				PackageNames: []string{"foo", "bar", "baz"},
			},
			[]string{"f*"},
			util.Set{"foo": "foo"},
		},
		{
			"return an array of matches",
			&context.Context{
				PackageNames: []string{"foo", "bar", "baz"},
			},
			[]string{"f*", "bar"},
			util.Set{"bar": "bar", "foo": "foo"},
		},
		{
			"return matches in the order the list were defined",
			&context.Context{
				PackageNames: []string{"foo", "bar", "baz"},
			},
			[]string{"*a*", "!f*"},
			util.Set{"bar": "bar", "baz": "baz"},
		},
	}

	for i, tc := range cases {
		t.Run(fmt.Sprintf("%d-%s", i, tc.Name), func(t *testing.T) {
			actual, err := getScopedPackages(tc.Ctx, tc.Patttern)
			if err != nil {
				t.Fatalf("invalid scope parse: %#v", err)
			}
			assert.EqualValues(t, tc.Expected, actual)
		})
	}
}
