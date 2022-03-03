package run

import (
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/mitchellh/cli"
	"github.com/vercel/turborepo/cli/internal/context"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/util"

	"github.com/stretchr/testify/assert"
)

func TestParseConfig(t *testing.T) {
	defaultCwd, err := os.Getwd()
	if err != nil {
		t.Errorf("failed to get cwd: %v", err)
	}
	defaultCacheFolder := filepath.Join(defaultCwd, filepath.FromSlash("node_modules/.cache/turbo"))
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
				cwd:                 defaultCwd,
				cacheFolder:         defaultCacheFolder,
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
				cwd:                 defaultCwd,
				cacheFolder:         defaultCacheFolder,
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
				cwd:                 defaultCwd,
				cacheFolder:         defaultCacheFolder,
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
				cwd:                 defaultCwd,
				cacheFolder:         defaultCacheFolder,
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
				cwd:                 defaultCwd,
				cacheFolder:         defaultCacheFolder,
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
				cwd:                 defaultCwd,
				cacheFolder:         defaultCacheFolder,
				passThroughArgs:     []string{},
			},
		},
	}

	ui := &cli.BasicUi{
		Reader:      os.Stdin,
		Writer:      os.Stdout,
		ErrorWriter: os.Stderr,
	}

	for i, tc := range cases {
		t.Run(fmt.Sprintf("%d-%s", i, tc.Name), func(t *testing.T) {

			actual, err := parseRunArgs(tc.Args, ui)
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

func TestGetTargetsFromArguments(t *testing.T) {
	type args struct {
		arguments  []string
		configJson *fs.TurboConfigJSON
	}
	tests := []struct {
		name    string
		args    args
		want    []string
		wantErr bool
	}{
		{
			name: "handles one defined target",
			args: args{
				arguments: []string{"build"},
				configJson: &fs.TurboConfigJSON{
					Pipeline: map[string]fs.Pipeline{
						"build":      {},
						"test":       {},
						"thing#test": {},
					},
				},
			},
			want:    []string{"build"},
			wantErr: false,
		},
		{
			name: "handles multiple targets and ignores flags",
			args: args{
				arguments: []string{"build", "test", "--foo", "--bar"},
				configJson: &fs.TurboConfigJSON{
					Pipeline: map[string]fs.Pipeline{
						"build":      {},
						"test":       {},
						"thing#test": {},
					},
				},
			},
			want:    []string{"build", "test"},
			wantErr: false,
		},
		{
			name: "handles pass through arguments after -- ",
			args: args{
				arguments: []string{"build", "test", "--", "--foo", "build", "--cache-dir"},
				configJson: &fs.TurboConfigJSON{
					Pipeline: map[string]fs.Pipeline{
						"build":      {},
						"test":       {},
						"thing#test": {},
					},
				},
			},
			want:    []string{"build", "test"},
			wantErr: false,
		},
		{
			name: "handles unknown pipeline targets ",
			args: args{
				arguments: []string{"foo", "test", "--", "--foo", "build", "--cache-dir"},
				configJson: &fs.TurboConfigJSON{
					Pipeline: map[string]fs.Pipeline{
						"build":      {},
						"test":       {},
						"thing#test": {},
					},
				},
			},
			want:    nil,
			wantErr: true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := getTargetsFromArguments(tt.args.arguments, tt.args.configJson)
			if (err != nil) != tt.wantErr {
				t.Errorf("GetTargetsFromArguments() error = %v, wantErr %v", err, tt.wantErr)
				return
			}

			if !reflect.DeepEqual(got, tt.want) {
				t.Errorf("GetTargetsFromArguments() = %v, want %v", got, tt.want)
			}
		})
	}
}
