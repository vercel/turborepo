package run

import (
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/mitchellh/cli"
	"github.com/pyr-sh/dag"
	"github.com/vercel/turborepo/cli/internal/cache"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/runcache"
	"github.com/vercel/turborepo/cli/internal/scope"
	"github.com/vercel/turborepo/cli/internal/util"

	"github.com/stretchr/testify/assert"
)

func TestParseConfig(t *testing.T) {
	defaultCwd, err := fs.GetCwd()
	if err != nil {
		t.Errorf("failed to get cwd: %v", err)
	}
	defaultCacheFolder := defaultCwd.Join(filepath.FromSlash("node_modules/.cache/turbo"))
	cases := []struct {
		Name     string
		Args     []string
		Expected *Opts
	}{
		{
			"string flags",
			[]string{"foo"},
			&Opts{
				runOpts: runOpts{
					concurrency: 10,
				},
				cacheOpts: cache.Opts{
					Dir:     defaultCacheFolder,
					Workers: 10,
				},
				runcacheOpts: runcache.Opts{},
				scopeOpts:    scope.Opts{},
			},
		},
		{
			"scope",
			[]string{"foo", "--scope=foo", "--scope=blah"},
			&Opts{
				runOpts: runOpts{
					concurrency: 10,
				},
				cacheOpts: cache.Opts{
					Dir:     defaultCacheFolder,
					Workers: 10,
				},
				runcacheOpts: runcache.Opts{},
				scopeOpts: scope.Opts{
					LegacyFilter: scope.LegacyFilter{
						Entrypoints: []string{"foo", "blah"},
					},
				},
			},
		},
		{
			"concurrency",
			[]string{"foo", "--concurrency=12"},
			&Opts{
				runOpts: runOpts{
					concurrency: 12,
				},
				cacheOpts: cache.Opts{
					Dir:     defaultCacheFolder,
					Workers: 10,
				},
				runcacheOpts: runcache.Opts{},
				scopeOpts:    scope.Opts{},
			},
		},
		{
			"graph",
			[]string{"foo", "--graph=g.png"},
			&Opts{
				runOpts: runOpts{
					concurrency: 10,
					dotGraph:    "g.png",
				},
				cacheOpts: cache.Opts{
					Dir:     defaultCacheFolder,
					Workers: 10,
				},
				runcacheOpts: runcache.Opts{},
				scopeOpts:    scope.Opts{},
			},
		},
		{
			"passThroughArgs",
			[]string{"foo", "--graph=g.png", "--", "--boop", "zoop"},
			&Opts{
				runOpts: runOpts{
					concurrency:     10,
					dotGraph:        "g.png",
					passThroughArgs: []string{"--boop", "zoop"},
				},
				cacheOpts: cache.Opts{
					Dir:     defaultCacheFolder,
					Workers: 10,
				},
				runcacheOpts: runcache.Opts{},
				scopeOpts:    scope.Opts{},
			},
		},
		{
			"force",
			[]string{"foo", "--force"},
			&Opts{
				runOpts: runOpts{
					concurrency: 10,
				},
				cacheOpts: cache.Opts{
					Dir:     defaultCacheFolder,
					Workers: 10,
				},
				runcacheOpts: runcache.Opts{
					SkipReads: true,
				},
				scopeOpts: scope.Opts{},
			},
		},
		{
			"remote-only",
			[]string{"foo", "--remote-only"},
			&Opts{
				runOpts: runOpts{
					concurrency: 10,
				},
				cacheOpts: cache.Opts{
					Dir:            defaultCacheFolder,
					Workers:        10,
					SkipFilesystem: true,
				},
				runcacheOpts: runcache.Opts{},
				scopeOpts:    scope.Opts{},
			},
		},
		{
			"no-cache",
			[]string{"foo", "--no-cache"},
			&Opts{
				runOpts: runOpts{
					concurrency: 10,
				},
				cacheOpts: cache.Opts{
					Dir:     defaultCacheFolder,
					Workers: 10,
				},
				runcacheOpts: runcache.Opts{
					SkipWrites: true,
				},
				scopeOpts: scope.Opts{},
			},
		},
		{
			"Empty passThroughArgs",
			[]string{"foo", "--graph=g.png", "--"},
			&Opts{
				runOpts: runOpts{
					concurrency:     10,
					dotGraph:        "g.png",
					passThroughArgs: []string{},
				},
				cacheOpts: cache.Opts{
					Dir:     defaultCacheFolder,
					Workers: 10,
				},
				runcacheOpts: runcache.Opts{},
				scopeOpts:    scope.Opts{},
			},
		},
		{
			"can specify filter patterns",
			[]string{"foo", "--filter=bar", "--filter=...[main]"},
			&Opts{
				runOpts: runOpts{
					concurrency: 10,
				},
				cacheOpts: cache.Opts{
					Dir:     defaultCacheFolder,
					Workers: 10,
				},
				runcacheOpts: runcache.Opts{},
				scopeOpts: scope.Opts{
					FilterPatterns: []string{"bar", "...[main]"},
				},
			},
		},
		{
			"continue on errors",
			[]string{"foo", "--continue"},
			&Opts{
				runOpts: runOpts{
					continueOnError: true,
					concurrency:     10,
				},
				cacheOpts: cache.Opts{
					Dir:     defaultCacheFolder,
					Workers: 10,
				},
				runcacheOpts: runcache.Opts{},
				scopeOpts:    scope.Opts{},
			},
		},
	}

	ui := &cli.BasicUi{
		Reader:      os.Stdin,
		Writer:      os.Stdout,
		ErrorWriter: os.Stderr,
	}

	cf := &config.Config{
		Cwd:    defaultCwd,
		Token:  "some-token",
		TeamId: "my-team",
		Cache: &config.CacheConfig{
			Workers: 10,
		},
	}
	for i, tc := range cases {
		t.Run(fmt.Sprintf("%d-%s", i, tc.Name), func(t *testing.T) {

			actual, err := parseRunArgs(tc.Args, cf, ui)
			if err != nil {
				t.Fatalf("invalid parse: %#v", err)
			}
			assert.EqualValues(t, tc.Expected, actual)
		})
	}
}

func TestParseRunOptionsUsesCWDFlag(t *testing.T) {
	defaultCwd, err := fs.GetCwd()
	if err != nil {
		t.Errorf("failed to get cwd: %v", err)
	}
	cwd := defaultCwd.Join("zop")
	expected := &Opts{
		runOpts: runOpts{
			concurrency: 10,
		},
		cacheOpts: cache.Opts{
			Dir:     cwd.Join("node_modules", ".cache", "turbo"),
			Workers: 10,
		},
		runcacheOpts: runcache.Opts{},
		scopeOpts:    scope.Opts{},
	}

	ui := &cli.BasicUi{
		Reader:      os.Stdin,
		Writer:      os.Stdout,
		ErrorWriter: os.Stderr,
	}

	t.Run("accepts cwd argument", func(t *testing.T) {
		cf := &config.Config{
			Cwd:    cwd,
			Token:  "some-token",
			TeamId: "my-team",
			Cache: &config.CacheConfig{
				Workers: 10,
			},
		}
		// Note that the Run parsing actually ignores `--cwd=` arg since
		// the `--cwd=` is parsed when setting up the global Config. This value is
		// passed directly as an argument to the parser.
		// We still need to ensure run accepts cwd flag and doesn't error.
		actual, err := parseRunArgs([]string{"foo", "--cwd=zop"}, cf, ui)
		if err != nil {
			t.Fatalf("invalid parse: %#v", err)
		}
		assert.EqualValues(t, expected, actual)
	})

}

func TestGetTargetsFromArguments(t *testing.T) {
	type args struct {
		arguments []string
		pipeline  fs.Pipeline
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
				pipeline: map[string]fs.TaskDefinition{
					"build":      {},
					"test":       {},
					"thing#test": {},
				},
			},
			want:    []string{"build"},
			wantErr: false,
		},
		{
			name: "handles multiple targets and ignores flags",
			args: args{
				arguments: []string{"build", "test", "--foo", "--bar"},
				pipeline: map[string]fs.TaskDefinition{
					"build":      {},
					"test":       {},
					"thing#test": {},
				},
			},
			want:    []string{"build", "test"},
			wantErr: false,
		},
		{
			name: "handles pass through arguments after -- ",
			args: args{
				arguments: []string{"build", "test", "--", "--foo", "build", "--cache-dir"},
				pipeline: map[string]fs.TaskDefinition{
					"build":      {},
					"test":       {},
					"thing#test": {},
				},
			},
			want:    []string{"build", "test"},
			wantErr: false,
		},
		{
			name: "handles unknown pipeline targets ",
			args: args{
				arguments: []string{"foo", "test", "--", "--foo", "build", "--cache-dir"},
				pipeline: map[string]fs.TaskDefinition{
					"build":      {},
					"test":       {},
					"thing#test": {},
				},
			},
			want:    nil,
			wantErr: true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := getTargetsFromArguments(tt.args.arguments, tt.args.pipeline)
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

func Test_dontSquashTasks(t *testing.T) {
	topoGraph := &dag.AcyclicGraph{}
	topoGraph.Add("a")
	topoGraph.Add("b")
	// no dependencies between packages

	pipeline := map[string]fs.TaskDefinition{
		"build": {
			Outputs:          []string{},
			TaskDependencies: []string{"generate"},
		},
		"generate": {
			Outputs: []string{},
		},
		"b#build": {
			Outputs: []string{},
		},
	}
	filteredPkgs := make(util.Set)
	filteredPkgs.Add("a")
	filteredPkgs.Add("b")
	rs := &runSpec{
		FilteredPkgs: filteredPkgs,
		Targets:      []string{"build"},
		Opts:         &Opts{},
	}
	engine, err := buildTaskGraph(topoGraph, pipeline, rs)
	if err != nil {
		t.Fatalf("failed to build task graph: %v", err)
	}
	toRun := engine.TaskGraph.Vertices()
	// 4 is the 3 tasks + root
	if len(toRun) != 4 {
		t.Errorf("expected 4 tasks, got %v", len(toRun))
	}
	for task := range pipeline {
		if _, ok := engine.Tasks[task]; !ok {
			t.Errorf("expected to find task %v in the task graph, but it is missing", task)
		}
	}
}

func Test_taskSelfRef(t *testing.T) {
	topoGraph := &dag.AcyclicGraph{}
	topoGraph.Add("a")
	// no dependencies between packages

	pipeline := map[string]fs.TaskDefinition{
		"build": {
			TaskDependencies: []string{"build"},
		},
	}
	filteredPkgs := make(util.Set)
	filteredPkgs.Add("a")
	rs := &runSpec{
		FilteredPkgs: filteredPkgs,
		Targets:      []string{"build"},
		Opts:         &Opts{},
	}
	_, err := buildTaskGraph(topoGraph, pipeline, rs)
	if err == nil {
		t.Fatalf("expected to failed to build task graph: %v", err)
	}
}
