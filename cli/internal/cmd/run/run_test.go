package run

import (
	// "fmt"
	// "os"
	// "path/filepath"
	// "reflect"
	// "testing"

	// "github.com/vercel/turborepo/cli/internal/fs"
	// "github.com/stretchr/testify/assert"
)

// func TestParseConfig(t *testing.T) {
// 	defaultCwd, err := os.Getwd()
// 	if err != nil {
// 		t.Errorf("failed to get Cwd: %v", err)
// 	}
// 	defaultCacheDir := filepath.Join(defaultCwd, filepath.FromSlash("node_modules/.Cache/turbo"))
// 	cases := []struct {
// 		Name     string
// 		Args     []string
// 		Expected *RunOptions
// 	}{
// 		{
// 			"string flags",
// 			[]string{"foo"},
// 			&RunOptions{
// 				IncludeDependents: true,
// 				Stream:            true,
// 				Bail:              true,
// 				Graph:             false,
// 				DotGraph:          "",
// 				Concurrency:       10,
// 				IncludeDeps:       false,
// 				NoCache:           false,
// 				Force:             false,
// 				Profile:           "",
// 				Cwd:               defaultCwd,
// 				CacheDir:          defaultCacheDir,
// 			},
// 		},
// 		{
// 			"cwd",
// 			[]string{"foo", "--cwd=zop"},
// 			&RunOptions{
// 				IncludeDependents: true,
// 				Stream:            true,
// 				Bail:              true,
// 				Graph:             false,
// 				DotGraph:          "",
// 				Concurrency:       10,
// 				IncludeDeps:       false,
// 				NoCache:           false,
// 				Force:             false,
// 				Profile:           "",
// 				Cwd:               "zop",
// 				CacheDir:          filepath.FromSlash("zop/node_modules/.Cache/turbo"),
// 			},
// 		},
// 		{
// 			"scope",
// 			[]string{"foo", "--scope=foo", "--scope=blah"},
// 			&RunOptions{
// 				IncludeDependents: true,
// 				Stream:            true,
// 				Bail:              true,
// 				Graph:             false,
// 				DotGraph:          "",
// 				Concurrency:       10,
// 				IncludeDeps:       false,
// 				NoCache:           false,
// 				Force:             false,
// 				Profile:           "",
// 				Scope:             []string{"foo", "blah"},
// 				Cwd:               defaultCwd,
// 				CacheDir:          defaultCacheDir,
// 			},
// 		},
// 		{
// 			"concurrency",
// 			[]string{"foo", "--concurrency=12"},
// 			&RunOptions{
// 				IncludeDependents: true,
// 				Stream:            true,
// 				Bail:              true,
// 				Graph:             false,
// 				DotGraph:          "",
// 				Concurrency:       12,
// 				IncludeDeps:       false,
// 				NoCache:           false,
// 				Force:             false,
// 				Profile:           "",
// 				Cwd:               defaultCwd,
// 				CacheDir:          defaultCacheDir,
// 			},
// 		},
// 		{
// 			"graph",
// 			[]string{"foo", "--graph=g.png"},
// 			&RunOptions{
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
// 				CacheDir:          defaultCacheDir,
// 			},
// 		},
// 		{
// 			"passThroughArgs",
// 			[]string{"foo", "--graph=g.png", "--", "--boop", "zoop"},
// 			&RunOptions{
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
// 				CacheDir:          defaultCacheDir,
// 				PassThroughArgs:   []string{"--boop", "zoop"},
// 			},
// 		},
// 		{
// 			"Empty passThroughArgs",
// 			[]string{"foo", "--graph=g.png", "--"},
// 			&RunOptions{
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
// 				CacheDir:          defaultCacheDir,
// 				PassThroughArgs:   []string{},
// 			},
// 		},
// 		{
// 			"since and scope imply including dependencies for backwards compatibility",
// 			[]string{"foo", "--scope=bar", "--since=some-ref"},
// 			&RunOptions{
// 				IncludeDependents: true,
// 				Stream:            true,
// 				Bail:              true,
// 				Graph:             true,
// 				DotGraph:          "",
// 				Concurrency:       10,
// 				IncludeDeps:       true,
// 				NoCache:           false,
// 				Cwd:               defaultCwd,
// 				CacheDir:          defaultCacheDir,
// 				Scope:             []string{"bar"},
// 				Since:             "some-ref",
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

// func TestGetTargetsFromArguments(t *testing.T) {
// 	type args struct {
// 		arguments  []string
// 		configJson *fs.TurboConfigJSON
// 	}
// 	tests := []struct {
// 		name    string
// 		args    args
// 		want    []string
// 		wantErr bool
// 	}{
// 		{
// 			name: "handles one defined target",
// 			args: args{
// 				arguments: []string{"build"},
// 				configJson: &fs.TurboConfigJSON{
// 					Pipeline: map[string]fs.Pipeline{
// 						"build":      {},
// 						"test":       {},
// 						"thing#test": {},
// 					},
// 				},
// 			},
// 			want:    []string{"build"},
// 			wantErr: false,
// 		},
// 		{
// 			name: "handles multiple targets and ignores flags",
// 			args: args{
// 				arguments: []string{"build", "test", "--foo", "--bar"},
// 				configJson: &fs.TurboConfigJSON{
// 					Pipeline: map[string]fs.Pipeline{
// 						"build":      {},
// 						"test":       {},
// 						"thing#test": {},
// 					},
// 				},
// 			},
// 			want:    []string{"build", "test"},
// 			wantErr: false,
// 		},
// 		{
// 			name: "handles pass through arguments after -- ",
// 			args: args{
// 				arguments: []string{"build", "test", "--", "--foo", "build", "--Cache-dir"},
// 				configJson: &fs.TurboConfigJSON{
// 					Pipeline: map[string]fs.Pipeline{
// 						"build":      {},
// 						"test":       {},
// 						"thing#test": {},
// 					},
// 				},
// 			},
// 			want:    []string{"build", "test"},
// 			wantErr: false,
// 		},
// 		{
// 			name: "handles unknown pipeline targets ",
// 			args: args{
// 				arguments: []string{"foo", "test", "--", "--foo", "build", "--Cache-dir"},
// 				configJson: &fs.TurboConfigJSON{
// 					Pipeline: map[string]fs.Pipeline{
// 						"build":      {},
// 						"test":       {},
// 						"thing#test": {},
// 					},
// 				},
// 			},
// 			want:    nil,
// 			wantErr: true,
// 		},
// 	}

// 	for _, tt := range tests {
// 		t.Run(tt.name, func(t *testing.T) {
// 			got, err := getTargetsFromArguments(tt.args.arguments, tt.args.configJson)
// 			if (err != nil) != tt.wantErr {
// 				t.Errorf("GetTargetsFromArguments() error = %v, wantErr %v", err, tt.wantErr)
// 				return
// 			}

// 			if !reflect.DeepEqual(got, tt.want) {
// 				t.Errorf("GetTargetsFromArguments() = %v, want %v", got, tt.want)
// 			}
// 		})
// 	}
// }
