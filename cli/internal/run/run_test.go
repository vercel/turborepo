package run

import (
	"fmt"
	"path/filepath"
	"runtime"
	"testing"

	"github.com/pyr-sh/dag"
	"github.com/spf13/pflag"
	"github.com/vercel/turborepo/cli/internal/cache"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/runcache"
	"github.com/vercel/turborepo/cli/internal/scope"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"

	"github.com/stretchr/testify/assert"
)

func TestParseConfig(t *testing.T) {
	cpus := runtime.NumCPU()
	defaultCwd, err := fs.GetCwd()
	if err != nil {
		t.Errorf("failed to get cwd: %v", err)
	}
	defaultCacheFolder := defaultCwd.Join(filepath.FromSlash("node_modules/.cache/turbo"))
	cases := []struct {
		Name          string
		Args          []string
		Expected      *Opts
		ExpectedTasks []string
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
			[]string{"foo"},
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
			[]string{"foo"},
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
			[]string{"foo"},
		},
		{
			"concurrency percent",
			[]string{"foo", "--concurrency=100%"},
			&Opts{
				runOpts: runOpts{
					concurrency: cpus,
				},
				cacheOpts: cache.Opts{
					Dir:     defaultCacheFolder,
					Workers: 10,
				},
				runcacheOpts: runcache.Opts{},
				scopeOpts:    scope.Opts{},
			},
			[]string{"foo"},
		},
		{
			"graph file",
			[]string{"foo", "--graph=g.png"},
			&Opts{
				runOpts: runOpts{
					concurrency: 10,
					graphFile:   "g.png",
					graphDot:    false,
				},
				cacheOpts: cache.Opts{
					Dir:     defaultCacheFolder,
					Workers: 10,
				},
				runcacheOpts: runcache.Opts{},
				scopeOpts:    scope.Opts{},
			},
			[]string{"foo"},
		},
		{
			"graph default",
			[]string{"foo", "--graph"},
			&Opts{
				runOpts: runOpts{
					concurrency: 10,
					graphFile:   "",
					graphDot:    true,
				},
				cacheOpts: cache.Opts{
					Dir:     defaultCacheFolder,
					Workers: 10,
				},
				runcacheOpts: runcache.Opts{},
				scopeOpts:    scope.Opts{},
			},
			[]string{"foo"},
		},
		{
			"passThroughArgs",
			[]string{"foo", "--graph=g.png", "--", "--boop", "zoop"},
			&Opts{
				runOpts: runOpts{
					concurrency:     10,
					graphFile:       "g.png",
					graphDot:        false,
					passThroughArgs: []string{"--boop", "zoop"},
				},
				cacheOpts: cache.Opts{
					Dir:     defaultCacheFolder,
					Workers: 10,
				},
				runcacheOpts: runcache.Opts{},
				scopeOpts:    scope.Opts{},
			},
			[]string{"foo"},
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
			[]string{"foo"},
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
			[]string{"foo"},
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
			[]string{"foo"},
		},
		{
			"Empty passThroughArgs",
			[]string{"foo", "--graph=g.png", "--"},
			&Opts{
				runOpts: runOpts{
					concurrency:     10,
					graphFile:       "g.png",
					graphDot:        false,
					passThroughArgs: []string{},
				},
				cacheOpts: cache.Opts{
					Dir:     defaultCacheFolder,
					Workers: 10,
				},
				runcacheOpts: runcache.Opts{},
				scopeOpts:    scope.Opts{},
			},
			[]string{"foo"},
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
			[]string{"foo"},
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
			[]string{"foo"},
		},
		{
			"relative cache dir",
			[]string{"foo", "--continue", "--cache-dir=bar"},
			&Opts{
				runOpts: runOpts{
					continueOnError: true,
					concurrency:     10,
				},
				cacheOpts: cache.Opts{
					Dir:     defaultCwd.Join("bar"),
					Workers: 10,
				},
				runcacheOpts: runcache.Opts{},
				scopeOpts:    scope.Opts{},
			},
			[]string{"foo"},
		},
		{
			"absolute cache dir",
			[]string{"foo", "--continue", "--cache-dir=" + defaultCwd.Join("bar").ToString()},
			&Opts{
				runOpts: runOpts{
					continueOnError: true,
					concurrency:     10,
				},
				cacheOpts: cache.Opts{
					Dir:     defaultCwd.Join("bar"),
					Workers: 10,
				},
				runcacheOpts: runcache.Opts{},
				scopeOpts:    scope.Opts{},
			},
			[]string{"foo"},
		},
	}

	cf := &config.Config{
		Cwd: defaultCwd,
		Cache: &config.CacheConfig{
			Workers: 10,
		},
	}
	for i, tc := range cases {
		t.Run(fmt.Sprintf("%d-%s", i, tc.Name), func(t *testing.T) {
			flags := pflag.NewFlagSet("test-flags", pflag.ExitOnError)
			opts := optsFromFlags(flags, cf)
			err := flags.Parse(tc.Args)
			remainingArgs := flags.Args()
			tasks, passThroughArgs := parseTasksAndPassthroughArgs(remainingArgs, flags)
			opts.runOpts.passThroughArgs = passThroughArgs
			if err != nil {
				t.Fatalf("invalid parse: %#v", err)
			}
			assert.EqualValues(t, tc.Expected, opts)
			assert.EqualValues(t, tc.ExpectedTasks, tasks)
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

	t.Run("accepts cwd argument", func(t *testing.T) {
		cf := &config.Config{
			Cwd: cwd,
			Cache: &config.CacheConfig{
				Workers: 10,
			},
		}
		flags := pflag.NewFlagSet("test-flags", pflag.ExitOnError)
		opts := optsFromFlags(flags, cf)
		err := flags.Parse([]string{"foo", "--cwd=zop"})
		// Note that the Run parsing actually ignores `--cwd=` arg since
		// the `--cwd=` is parsed when setting up the global Config. This value is
		// passed directly as an argument to the parser.
		// We still need to ensure run accepts cwd flag and doesn't error.
		if err != nil {
			t.Fatalf("invalid parse: %#v", err)
		}
		assert.EqualValues(t, expected, opts)
	})

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

func TestUsageText(t *testing.T) {
	defaultCwd, err := fs.GetCwd()
	if err != nil {
		t.Fatalf("failed to get cwd: %v", err)
	}
	cf := &config.Config{
		Cwd: defaultCwd,
		Cache: &config.CacheConfig{
			Workers: 10,
		},
	}
	output := ui.Default()
	cmd := &RunCommand{
		Config: cf,
		UI:     output,
	}
	// just ensure it doesn't panic for now
	usage := cmd.Help()
	assert.NotEmpty(t, usage, "expected usage text")
}
