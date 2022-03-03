package run

import (
	"fmt"
	"reflect"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/vercel/turborepo/cli/internal/context"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/util"
)

// func TestParseConfig(t *testing.T) {
// 	defaultCwd, err := os.Getwd()
// 	if err != nil {
// 		t.Errorf("failed to get cwd: %v", err)
// 	}
// 	defaultCacheFolder := filepath.Join(defaultCwd, filepath.FromSlash("node_modules/.cache/turbo"))
// 	cases := []struct {
// 		Name     string
// 		Args     []string
// 		Expected *run.RunOptions
// 	}{
// 		{
// 			"string flags",
// 			[]string{"foo"},
// 			&run.RunOptions{
// 				IncludeDependents: true,
// 				Stream:            true,
// 				Bail:              true,
// 				DotGraph:          "",
// 				Concurrency:       10,
// 				IncludeDeps:       false,
// 				NoCache:           false,
// 				Force:             false,
// 				Profile:           "",
// 				Cwd:               defaultCwd,
// 				CacheDir:          defaultCacheFolder,
// 			},
// 		},
// 		{
// 			"cwd",
// 			[]string{"foo", "--cwd=zop"},
// 			&run.RunOptions{
// 				IncludeDependents: true,
// 				Stream:            true,
// 				Bail:              true,
// 				DotGraph:          "",
// 				Concurrency:       10,
// 				IncludeDeps:       false,
// 				NoCache:           false,
// 				Force:             false,
// 				Profile:           "",
// 				Cwd:               "zop",
// 				CacheDir:          filepath.FromSlash("zop/node_modules/.cache/turbo"),
// 			},
// 		},
// 		{
// 			"scope",
// 			[]string{"foo", "--scope=foo", "--scope=blah"},
// 			&run.RunOptions{
// 				IncludeDependents: true,
// 				Stream:            true,
// 				Bail:              true,
// 				DotGraph:          "",
// 				Concurrency:       10,
// 				IncludeDeps:       false,
// 				NoCache:           false,
// 				Force:             false,
// 				Profile:           "",
// 				Scope:             []string{"foo", "blah"},
// 				Cwd:               defaultCwd,
// 				CacheDir:          defaultCacheFolder,
// 			},
// 		},
// 		{
// 			"concurrency",
// 			[]string{"foo", "--concurrency=12"},
// 			&run.RunOptions{
// 				IncludeDependents: true,
// 				Stream:            true,
// 				Bail:              true,
// 				DotGraph:          "",
// 				Concurrency:       12,
// 				IncludeDeps:       false,
// 				NoCache:           false,
// 				Force:             false,
// 				Profile:           "",
// 				Cwd:               defaultCwd,
// 				CacheDir:          defaultCacheFolder,
// 			},
// 		},
// 		{
// 			"graph",
// 			[]string{"foo", "-g --graph-path=g.png"},
// 			&run.RunOptions{
// 				IncludeDependents: true,
// 				Stream:            true,
// 				Bail:              true,
// 				Graph:             true,
// 				DotGraph:          "g.png",
// 				Concurrency:       10,
// 				IncludeDeps:       false,
// 				NoCache:           false,
// 				Force:             false,
// 				Profile:           "",
// 				Cwd:               defaultCwd,
// 				CacheDir:          defaultCacheFolder,
// 			},
// 		},
// 		{
// 			"passThroughArgs",
// 			[]string{"foo", "-g --graph-path=g.png", "--", "--boop", "zoop"},
// 			&run.RunOptions{
// 				IncludeDependents: true,
// 				Stream:            true,
// 				Bail:              true,
// 				Graph:             true,
// 				DotGraph:          "g.png",
// 				Concurrency:       10,
// 				IncludeDeps:       false,
// 				NoCache:           false,
// 				Force:             false,
// 				Profile:           "",
// 				Cwd:               defaultCwd,
// 				CacheDir:          defaultCacheFolder,
// 				PassThroughArgs:   []string{"--boop", "zoop"},
// 			},
// 		},
// 		{
// 			"Empty passThroughArgs",
// 			[]string{"foo", "-g --graph-path=g.png", "--"},
// 			&run.RunOptions{
// 				IncludeDependents: true,
// 				Stream:            true,
// 				Bail:              true,
// 				Graph:             true,
// 				DotGraph:          "g.png",
// 				Concurrency:       10,
// 				IncludeDeps:       false,
// 				NoCache:           false,
// 				Force:             false,
// 				Profile:           "",
// 				Cwd:               defaultCwd,
// 				CacheDir:          defaultCacheFolder,
// 				PassThroughArgs:   []string{},
// 			},
// 		},
// 	}

// 	ui := &cli.BasicUi{
// 		Reader:      os.Stdin,
// 		Writer:      os.Stdout,
// 		ErrorWriter: os.Stderr,
// 	}

// 	for i, tc := range cases {
// 		t.Run(fmt.Sprintf("%d-%s", i, tc.Name), func(t *testing.T) {
// 			actual, err := parseRunArgs(tc.Args, ui)
// 			if err != nil {
// 				t.Fatalf("invalid parse: %#v", err)
// 			}
// 			assert.EqualValues(t, actual, tc.Expected)
// 		})
// 	}
// }

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
