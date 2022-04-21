package run

import (
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/mitchellh/cli"
	"github.com/pyr-sh/dag"
	"github.com/vercel/turborepo/cli/internal/fs"
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
				cwd:                 defaultCwd.ToStringDuringMigration(),
				cacheFolder:         defaultCacheFolder.ToStringDuringMigration(),
				cacheHitLogsMode:    FullLogs,
				cacheMissLogsMode:   FullLogs,
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
				cwd:                 defaultCwd.ToStringDuringMigration(),
				cacheFolder:         defaultCacheFolder.ToStringDuringMigration(),
				cacheHitLogsMode:    FullLogs,
				cacheMissLogsMode:   FullLogs,
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
				cwd:                 defaultCwd.ToStringDuringMigration(),
				cacheFolder:         defaultCacheFolder.ToStringDuringMigration(),
				cacheHitLogsMode:    FullLogs,
				cacheMissLogsMode:   FullLogs,
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
				cwd:                 defaultCwd.ToStringDuringMigration(),
				cacheFolder:         defaultCacheFolder.ToStringDuringMigration(),
				cacheHitLogsMode:    FullLogs,
				cacheMissLogsMode:   FullLogs,
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
				cwd:                 defaultCwd.ToStringDuringMigration(),
				cacheFolder:         defaultCacheFolder.ToStringDuringMigration(),
				passThroughArgs:     []string{"--boop", "zoop"},
				cacheHitLogsMode:    FullLogs,
				cacheMissLogsMode:   FullLogs,
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
				cwd:                 defaultCwd.ToStringDuringMigration(),
				cacheFolder:         defaultCacheFolder.ToStringDuringMigration(),
				passThroughArgs:     []string{},
				cacheHitLogsMode:    FullLogs,
				cacheMissLogsMode:   FullLogs,
			},
		},
		{
			"can specify filter patterns",
			[]string{"foo", "--filter=bar", "--filter=...[main]"},
			&RunOptions{
				includeDependents: true,
				filterPatterns:    []string{"bar", "...[main]"},
				stream:            true,
				bail:              true,
				concurrency:       10,
				cache:             true,
				cwd:               defaultCwd.ToStringDuringMigration(),
				cacheFolder:       defaultCacheFolder.ToStringDuringMigration(),
				cacheHitLogsMode:  FullLogs,
				cacheMissLogsMode: FullLogs,
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

			actual, err := parseRunArgs(tc.Args, defaultCwd, ui)
			if err != nil {
				t.Fatalf("invalid parse: %#v", err)
			}
			assert.EqualValues(t, tc.Expected, actual)
		})
	}
}

func TestParseRunOptionsUsesCWDFlag(t *testing.T) {
	expected := &RunOptions{
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
		cacheHitLogsMode:    FullLogs,
		cacheMissLogsMode:   FullLogs,
	}

	ui := &cli.BasicUi{
		Reader:      os.Stdin,
		Writer:      os.Stdout,
		ErrorWriter: os.Stderr,
	}

	t.Run("accepts cwd argument", func(t *testing.T) {
		// Note that the Run parsing actually ignores `--cwd=` arg since
		// the `--cwd=` is parsed when setting up the global Config. This value is
		// passed directly as an argument to the parser.
		// We still need to ensure run accepts cwd flag and doesn't error.
		actual, err := parseRunArgs([]string{"foo", "--cwd=zop"}, "zop", ui)
		if err != nil {
			t.Fatalf("invalid parse: %#v", err)
		}
		assert.EqualValues(t, expected, actual)
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
					Pipeline: map[string]fs.TaskDefinition{
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
					Pipeline: map[string]fs.TaskDefinition{
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
					Pipeline: map[string]fs.TaskDefinition{
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
					Pipeline: map[string]fs.TaskDefinition{
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

func TestGetTargetsFromGlobArguments(t *testing.T) {
	type args struct {
		arguments  []string
		configJson *fs.TurboConfigJSON
	}
	pipelineConfig := map[string]fs.TaskDefinition{
		"alpha:aaa:check":       {},
		"alpha:aaa:update":      {},
		"alpha:aaa:build":       {},
		"alpha:bbb:check":       {},
		"alpha:bbb:build":       {},
		"alpha:ccc:iii:update":  {},
		"alpha:ccc:iii:build":   {},
		"alpha:ccc:jjj:check":   {},
		"alpha:ccc:jjj:build":   {},
		"beta:a*:update":        {},
		"beta:a*:build":         {},
		"charlie:ijk:xyz:check": {},
		"charlie:build":         {},
		"delta:check":           {},
		"echo:check":            {},
		"foxtrot:update":        {},
		"foxtrot:build":         {},
		"foxtrot:aaa:update":    {},
		"foxtrot:aaa:clean":     {},
		"foxtrot:aaa:456":       {},
		"foxtrot:123:clean":     {},
	}
	tests := []struct {
		name    string
		args    args
		want    []string
		wantErr bool
	}{
		{
			name: "handles single glob targets",
			args: args{
				arguments: []string{
					"*check",           // no results
					"alpha:*check",     // no results
					"alpha:*:build",    // alpha:aaa:build alpha:bbb:build
					"beta:*",           // no results
					"beta:a\\*:update", // beta:a*:update
					"foxtrot:*",        // foxtrot:build foxtrot:update
				},
				configJson: &fs.TurboConfigJSON{
					Pipeline: pipelineConfig,
				},
			},
			want:    []string{"alpha:aaa:build", "alpha:bbb:build", "beta:a*:update", "foxtrot:build", "foxtrot:update"},
			wantErr: false,
		},
		{
			name: "handles double glob targets",
			args: args{
				arguments: []string{
					"*:**:check",      // alpha:aaa:check alpha:bbb:check alpha:ccc:jjj:check charlie:ijk:xyz:check delta:check echo:check
					"alpha:**:update", // alpha:aaa:update alpha:ccc:iii:update
					"beta:**build",    // no results
				},
				configJson: &fs.TurboConfigJSON{
					Pipeline: pipelineConfig,
				},
			},
			want:    []string{"alpha:aaa:check", "alpha:aaa:update", "alpha:bbb:check", "alpha:ccc:iii:update", "alpha:ccc:jjj:check", "charlie:ijk:xyz:check", "delta:check", "echo:check"},
			wantErr: false,
		},
		{
			name: "handles single and double glob targets",
			args: args{
				arguments: []string{
					"foxtrot:**:*update", // foxtrot:aaa:update foxtrot:update
					"*:check",            // delta:check echo:check
					"alpha:ccc:**:build", // alpha:ccc:iii:build alpha:ccc:jjj:build
					"alpha:bbb:*",        // alpha:bbb:build alpha:bbb:check
				},
				configJson: &fs.TurboConfigJSON{
					Pipeline: pipelineConfig,
				},
			},
			want:    []string{"alpha:bbb:build", "alpha:bbb:check", "alpha:ccc:iii:build", "alpha:ccc:jjj:build", "delta:check", "echo:check", "foxtrot:aaa:update", "foxtrot:update"},
			wantErr: false,
		},
		{
			name: "handles wildcards, braces, and character class glob targets",
			args: args{
				arguments: []string{
					"alpha:{a*,b*}:check",             // alpha:aaa:check alpha:bbb:check
					"alpha:ccc:{iii,jjj}:build",       // alpha:ccc:iii:build alpha:ccc:jjj:build
					"foxtrot:[a-z][a-z][a-z]:[^0-9]*", // foxtrot:aaa:clean foxtrot:aaa:update
					"beta:a?:update",                  // beta:a*:update
				},
				configJson: &fs.TurboConfigJSON{
					Pipeline: pipelineConfig,
				},
			},
			want:    []string{"alpha:aaa:check", "alpha:bbb:check", "alpha:ccc:iii:build", "alpha:ccc:jjj:build", "beta:a*:update", "foxtrot:aaa:clean", "foxtrot:aaa:update"},
			wantErr: false,
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
		Opts:         &RunOptions{},
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
