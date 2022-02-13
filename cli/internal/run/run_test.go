package run

import (
	"fmt"
	"path/filepath"
	"testing"
	"github.com/vercel/turborepo/cli/internal/context"
	"github.com/vercel/turborepo/cli/internal/util"

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
				includeDependents:   true,
				stream:              true,
				bail:                true,
				dotGraph:            "",
				concurrency:         10,
				includeDependencies: false,
				cache:               true,
				forceExecution:      false,
				profile:             "",
				cacheFolder:         filepath.FromSlash("node_modules/.cache/turbo"),
			},
		},
		{
			"cwd",
			[]string{"foo", "--cwd=zop"},
			&RunOptions{
				includeDependents:   true,
				stream:              true,
				bail:                true,
				dotGraph:            "",
				concurrency:         10,
				includeDependencies: false,
				cache:               true,
				forceExecution:      false,
				profile:             "",
				cwd:                 "zop",
				cacheFolder:         filepath.FromSlash("zop/node_modules/.cache/turbo"),
			},
		},
		{
			"scope",
			[]string{"foo", "--scope=foo", "--scope=blah"},
			&RunOptions{
				includeDependents:   true,
				stream:              true,
				bail:                true,
				dotGraph:            "",
				concurrency:         10,
				includeDependencies: false,
				cache:               true,
				forceExecution:      false,
				profile:             "",
				scope:               []string{"foo", "blah"},
				cacheFolder:         filepath.FromSlash("node_modules/.cache/turbo"),
			},
		},
		{
			"concurrency",
			[]string{"foo", "--concurrency=12"},
			&RunOptions{
				includeDependents:   true,
				stream:              true,
				bail:                true,
				dotGraph:            "",
				concurrency:         12,
				includeDependencies: false,
				cache:               true,
				forceExecution:      false,
				profile:             "",
				cacheFolder:         filepath.FromSlash("node_modules/.cache/turbo"),
			},
		},
		{
			"graph",
			[]string{"foo", "--graph=g.png"},
			&RunOptions{
				includeDependents:   true,
				stream:              true,
				bail:                true,
				dotGraph:            "g.png",
				concurrency:         10,
				includeDependencies: false,
				cache:               true,
				forceExecution:      false,
				profile:             "",
				cacheFolder:         filepath.FromSlash("node_modules/.cache/turbo"),
			},
		},
		{
			"passThroughArgs",
			[]string{"foo", "--graph=g.png", "--", "--boop", "zoop"},
			&RunOptions{
				includeDependents:   true,
				stream:              true,
				bail:                true,
				dotGraph:            "g.png",
				concurrency:         10,
				includeDependencies: false,
				cache:               true,
				forceExecution:      false,
				profile:             "",
				cacheFolder:         filepath.FromSlash("node_modules/.cache/turbo"),
				passThroughArgs:     []string{"--boop", "zoop"},
			},
		},
		{
			"Empty passThroughArgs",
			[]string{"foo", "--graph=g.png", "--"},
			&RunOptions{
				includeDependents:   true,
				stream:              true,
				bail:                true,
				dotGraph:            "g.png",
				concurrency:         10,
				includeDependencies: false,
				cache:               true,
				forceExecution:      false,
				profile:             "",
				cacheFolder:         filepath.FromSlash("node_modules/.cache/turbo"),
				passThroughArgs:     []string{},
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
		Pattern  []string
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
			actual, err := getScopedPackages(tc.Ctx, tc.Pattern)
			if err != nil {
				t.Fatalf("invalid scope parse: %#v", err)
			}
			assert.EqualValues(t, tc.Expected, actual)
		})
	}

	t.Run(fmt.Sprintf("%d-%s", len(cases), "throws an error if no package matches the provided scope pattern"), func(t *testing.T) {
		_, err := getScopedPackages(&context.Context{PackageNames: []string{"foo", "bar"}}, []string{"baz"})
		assert.Error(t, err)
	})
}
