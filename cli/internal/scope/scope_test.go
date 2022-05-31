package scope

import (
	"fmt"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/hashicorp/go-hclog"
	"github.com/pyr-sh/dag"
	"github.com/vercel/turborepo/cli/internal/context"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"
)

type mockSCM struct {
	changed []string
}

func (m *mockSCM) ChangedFiles(_fromCommit string, _includeUntracked bool, _relativeTo string) ([]string, error) {
	return m.changed, nil
}

func TestResolvePackages(t *testing.T) {
	tui := ui.Default()
	logger := hclog.Default()
	//
	// app0 -
	//        \
	// app1 -> libA
	//              \
	//                > libB -> libD
	//              /
	//       app2 <
	//              \ libC
	//
	graph := dag.AcyclicGraph{}
	graph.Add("app0")
	graph.Add("app1")
	graph.Add("app2")
	graph.Add("libA")
	graph.Add("libB")
	graph.Add("libC")
	graph.Add("libD")
	graph.Connect(dag.BasicEdge("libA", "libB"))
	graph.Connect(dag.BasicEdge("libB", "libD"))
	graph.Connect(dag.BasicEdge("app0", "libA"))
	graph.Connect(dag.BasicEdge("app1", "libA"))
	graph.Connect(dag.BasicEdge("app2", "libB"))
	graph.Connect(dag.BasicEdge("app2", "libC"))
	packagesInfos := map[interface{}]*fs.PackageJSON{
		"app0": {
			Dir: "app/app0",
		},
		"app1": {
			Dir: "app/app1",
		},
		"app2": {
			Dir: "app/app2",
		},
		"libA": {
			Dir: "libs/libA",
		},
		"libB": {
			Dir: "libs/libB",
		},
		"libC": {
			Dir: "libs/libC",
		},
		"libD": {
			Dir: "libs/libD",
		},
	}
	packageNames := []string{}
	for name := range packagesInfos {
		packageNames = append(packageNames, name.(string))
	}

	testCases := []struct {
		name                string
		changed             []string
		expected            []string
		expectAllPackages   bool
		scope               []string
		since               string
		ignore              string
		globalDeps          []string
		includeDependencies bool
		includeDependents   bool
	}{
		{
			name:                "Just scope and dependencies",
			changed:             []string{},
			includeDependencies: true,
			scope:               []string{"app2"},
			expected:            []string{"app2", "libB", "libC", "libD"},
		},
		{
			name:     "One package changed",
			changed:  []string{"libs/libB/src/index.ts"},
			expected: []string{"libB"},
			since:    "dummy",
		},
		{
			name:     "An ignored package changed",
			changed:  []string{"libs/libB/src/index.ts"},
			expected: []string{},
			since:    "dummy",
			ignore:   "libs/libB/**/*.ts",
		},
		{
			// nothing in scope depends on the change
			name:                "unrelated library changed",
			changed:             []string{"libs/libC/src/index.ts"},
			expected:            []string{},
			since:               "dummy",
			scope:               []string{"app1"},
			includeDependencies: true, // scope implies include-dependencies
		},
		{
			// a dependent lib changed, scope implies include-dependencies,
			// so all deps of app1 get built
			name:                "dependency of scope changed",
			changed:             []string{"libs/libA/src/index.ts"},
			expected:            []string{"libA", "libB", "libD", "app1"},
			since:               "dummy",
			scope:               []string{"app1"},
			includeDependencies: true, // scope implies include-dependencies
		},
		{
			// a dependent lib changed, user explicitly asked to not build dependencies.
			// Since the package matching the scope had a changed dependency, we run it.
			// We don't include its dependencies because the user asked for no dependencies.
			// note: this is not yet supported by the CLI, as you cannot specify --include-dependencies=false
			name:                "dependency of scope changed, user asked to not include depedencies",
			changed:             []string{"libs/libA/src/index.ts"},
			expected:            []string{"app1"},
			since:               "dummy",
			scope:               []string{"app1"},
			includeDependencies: false,
		},
		{
			// a nested dependent lib changed, user explicitly asked to not build dependencies
			// note: this is not yet supported by the CLI, as you cannot specify --include-dependencies=false
			name:                "nested dependency of scope changed, user asked to not include dependencies",
			changed:             []string{"libs/libB/src/index.ts"},
			expected:            []string{"app1"},
			since:               "dummy",
			scope:               []string{"app1"},
			includeDependencies: false,
		},
		{
			name:       "global dependency changed, even though it was ignored, forcing a build of everything",
			changed:    []string{"libs/libB/src/index.ts"},
			expected:   []string{"app0", "app1", "app2", "libA", "libB", "libC", "libD"},
			since:      "dummy",
			ignore:     "libs/libB/**/*.ts",
			globalDeps: []string{"libs/**/*.ts"},
		},
		{
			name:                "an app changed, user asked for dependencies to build",
			changed:             []string{"app/app2/src/index.ts"},
			since:               "dummy",
			includeDependencies: true,
			expected:            []string{"app2", "libB", "libC", "libD"},
		},
		{
			name:              "a library changed, user asked for dependents to be built",
			changed:           []string{"libs/libB"},
			since:             "dummy",
			includeDependents: true,
			expected:          []string{"app0", "app1", "app2", "libA", "libB"},
		},
		{
			// no changes, no base to compare against, defaults to everything
			name:              "no changes or scope specified, build everything",
			since:             "",
			expected:          []string{"app0", "app1", "app2", "libA", "libB", "libC", "libD"},
			expectAllPackages: true,
		},
		{
			// a dependent library changed, no deps beyond the scope are build
			// "libB" is still built because it is a dependent within the scope, but libB's dependents
			// are skipped
			name:                "a dependent library changed, build up to scope",
			changed:             []string{"libs/libD/src/index.ts"},
			since:               "dummy",
			scope:               []string{"libB"},
			expected:            []string{"libB", "libD"},
			includeDependencies: true, // scope implies include-dependencies
		},
		{
			name:              "library change, no scope",
			changed:           []string{"libs/libA/src/index.ts"},
			expected:          []string{"libA", "app0", "app1"},
			includeDependents: true,
			since:             "dummy",
		},
	}
	for i, tc := range testCases {
		t.Run(fmt.Sprintf("test #%v %v", i, tc.name), func(t *testing.T) {
			scm := &mockSCM{
				changed: tc.changed,
			}
			pkgs, isAllPackages, err := ResolvePackages(&Opts{
				LegacyFilter: LegacyFilter{
					Entrypoints:         tc.scope,
					Since:               tc.since,
					IncludeDependencies: tc.includeDependencies,
					SkipDependents:      !tc.includeDependents,
				},
				IgnorePatterns:    []string{tc.ignore},
				GlobalDepPatterns: tc.globalDeps,
			}, filepath.FromSlash("/dummy/repo/root"), scm, &context.Context{
				PackageInfos:     packagesInfos,
				PackageNames:     packageNames,
				TopologicalGraph: graph,
			}, tui, logger)
			if err != nil {
				t.Errorf("expected no error, got %v", err)
			}
			expected := make(util.Set)
			for _, pkg := range tc.expected {
				expected.Add(pkg)
			}
			if !reflect.DeepEqual(pkgs, expected) {
				t.Errorf("ResolvePackages got %v, want %v", pkgs, expected)
			}
			if isAllPackages != tc.expectAllPackages {
				t.Errorf("isAllPackages got %v, want %v", isAllPackages, tc.expectAllPackages)
			}
		})
	}
}
