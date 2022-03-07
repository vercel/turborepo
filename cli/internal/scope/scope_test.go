package scope

import (
	"fmt"
	"reflect"
	"strings"
	"testing"

	"github.com/hashicorp/go-hclog"
	"github.com/pyr-sh/dag"
	"github.com/stretchr/testify/assert"
	"github.com/vercel/turborepo/cli/internal/context"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/ui"
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

type mockSCM struct {
	changed []string
}

func (m *mockSCM) ChangedFiles(fromCommit string, includeUntracked bool, relativeTo string) []string {
	changed := []string{}
	for _, change := range m.changed {
		if strings.HasPrefix(change, relativeTo) {
			changed = append(changed, change)
		}
	}
	return changed
}

func TestResolvePackages(t *testing.T) {
	tui := ui.Default()
	logger := hclog.Default()
	// app1 -> libA -> libB
	//
	//        / libB
	// app2 <
	//        \ libC
	//
	graph := dag.AcyclicGraph{}
	graph.Add("app1")
	graph.Add("app2")
	graph.Add("libA")
	graph.Add("libB")
	graph.Add("libC")
	graph.Connect(dag.BasicEdge("libA", "libB"))
	graph.Connect(dag.BasicEdge("app1", "libA"))
	graph.Connect(dag.BasicEdge("app2", "libB"))
	graph.Connect(dag.BasicEdge("app2", "libC"))
	scc := dag.StronglyConnected(&graph.Graph)
	packagesInfos := map[interface{}]*fs.PackageJSON{
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
	}
	packageNames := []string{}
	for name := range packagesInfos {
		packageNames = append(packageNames, name.(string))
	}

	testCases := []struct {
		changed             []string
		expected            []string
		scope               []string
		since               string
		ignore              string
		globalDeps          []string
		includeDependencies bool
		includeDependents   bool
	}{
		{
			changed:  []string{"libs/libB/src/index.ts"},
			expected: []string{"libB"},
			since:    "dummy",
		},
		{
			changed:  []string{"libs/libB/src/index.ts"},
			expected: []string{},
			since:    "dummy",
			ignore:   "libs/libB/**/*.ts",
		},
		{
			// a non-dependent lib changed
			changed:  []string{"libs/libC/src/index.ts"},
			expected: []string{},
			since:    "dummy",
			scope:    []string{"app1"},
		},
		{
			changed: []string{"libs/libB/src/index.ts"},
			// expect everything, global changed, no scope
			expected:   []string{"app1", "app2", "libA", "libB", "libC"},
			since:      "dummy",
			ignore:     "libs/libB/**/*.ts",
			globalDeps: []string{"libs/**/*.ts"},
		},
		{
			changed:             []string{"app/app2/src/index.ts"},
			since:               "dummy",
			includeDependencies: true,
			expected:            []string{"app2", "libB", "libC"},
		},
		{
			changed:           []string{"libs/libB"},
			since:             "dummy",
			includeDependents: true,
			expected:          []string{"app1", "app2", "libA", "libB"},
		},
		{
			// no changes, no base to compare against, defaults to everything
			since:    "",
			expected: []string{"app1", "app2", "libA", "libB", "libC"},
		},
	}
	for i, tc := range testCases {
		t.Run(fmt.Sprintf("test #%v", i), func(t *testing.T) {
			scm := &mockSCM{
				changed: tc.changed,
			}
			pkgs, err := ResolvePackages(&Opts{
				Patterns:            tc.scope,
				Since:               tc.since,
				IgnorePatterns:      []string{tc.ignore},
				GlobalDepPatterns:   tc.globalDeps,
				IncludeDependencies: tc.includeDependencies,
				IncludeDependents:   tc.includeDependents,
			}, scm, &context.Context{
				PackageInfos:     packagesInfos,
				PackageNames:     packageNames,
				TopologicalGraph: graph,
				SCC:              scc,
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
		})
	}
}
