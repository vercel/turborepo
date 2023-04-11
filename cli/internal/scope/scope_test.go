package scope

import (
	"fmt"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/hashicorp/go-hclog"
	"github.com/pyr-sh/dag"
	"github.com/vercel/turbo/cli/internal/context"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/lockfile"
	"github.com/vercel/turbo/cli/internal/packagemanager"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/ui"
	"github.com/vercel/turbo/cli/internal/util"
	"github.com/vercel/turbo/cli/internal/workspace"
)

type mockSCM struct {
	changed  []string
	contents map[string][]byte
}

func (m *mockSCM) ChangedFiles(_fromCommit string, _toCommit string, _relativeTo string) ([]string, error) {
	return m.changed, nil
}

func (m *mockSCM) PreviousContent(fromCommit string, filePath string) ([]byte, error) {
	contents, ok := m.contents[filePath]
	if !ok {
		return nil, fmt.Errorf("No contents found")
	}
	return contents, nil
}

type mockLockfile struct {
	globalChange bool
	versions     map[string]string
	allDeps      map[string]map[string]string
}

func (m *mockLockfile) ResolvePackage(workspacePath turbopath.AnchoredUnixPath, name string, version string) (lockfile.Package, error) {
	resolvedVersion, ok := m.versions[name]
	if ok {
		key := fmt.Sprintf("%s%s", name, version)
		return lockfile.Package{Key: key, Version: resolvedVersion, Found: true}, nil
	}
	return lockfile.Package{Found: false}, nil
}

func (m *mockLockfile) AllDependencies(key string) (map[string]string, bool) {
	deps, ok := m.allDeps[key]
	return deps, ok
}

func (m *mockLockfile) Encode(w io.Writer) error {
	return nil
}

func (m *mockLockfile) GlobalChange(other lockfile.Lockfile) bool {
	return m.globalChange || (other != nil && other.(*mockLockfile).globalChange)
}

func (m *mockLockfile) Patches() []turbopath.AnchoredUnixPath {
	return nil
}

func (m *mockLockfile) Subgraph(workspaces []turbopath.AnchoredSystemPath, packages []string) (lockfile.Lockfile, error) {
	return nil, nil
}

var _ (lockfile.Lockfile) = (*mockLockfile)(nil)

func TestResolvePackages(t *testing.T) {
	cwd, err := os.Getwd()
	if err != nil {
		t.Fatalf("cwd: %v", err)
	}
	root, err := fs.GetCwd(cwd)
	if err != nil {
		t.Fatalf("cwd: %v", err)
	}
	tui := ui.Default()
	logger := hclog.Default()
	// Dependency graph:
	//
	// app0 -
	//        \
	// app1 -> libA
	//              \
	//                > libB -> libD
	//              /
	//       app2 <
	//              \
	//                > libC
	//              /
	//     app2-a <
	//
	// Filesystem layout:
	//
	// app/
	//   app0
	//   app1
	//   app2
	//   app2-a
	// libs/
	//   libA
	//   libB
	//   libC
	//   libD
	graph := dag.AcyclicGraph{}
	graph.Add("app0")
	graph.Add("app1")
	graph.Add("app2")
	graph.Add("app2-a")
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
	graph.Connect(dag.BasicEdge("app2-a", "libC"))
	workspaceInfos := workspace.Catalog{
		PackageJSONs: map[string]*fs.PackageJSON{
			"//": {
				Dir:                    turbopath.AnchoredSystemPath("").ToSystemPath(),
				UnresolvedExternalDeps: map[string]string{"global": "2"},
				TransitiveDeps:         []lockfile.Package{{Key: "global2", Version: "2", Found: true}},
			},
			"app0": {
				Dir:                    turbopath.AnchoredUnixPath("app/app0").ToSystemPath(),
				Name:                   "app0",
				UnresolvedExternalDeps: map[string]string{"app0-dep": "2"},
				TransitiveDeps: []lockfile.Package{
					{Key: "app0-dep2", Version: "2", Found: true},
					{Key: "app0-util2", Version: "2", Found: true},
				},
			},
			"app1": {
				Dir:  turbopath.AnchoredUnixPath("app/app1").ToSystemPath(),
				Name: "app1",
			},
			"app2": {
				Dir:  turbopath.AnchoredUnixPath("app/app2").ToSystemPath(),
				Name: "app2",
			},
			"app2-a": {
				Dir:  turbopath.AnchoredUnixPath("app/app2-a").ToSystemPath(),
				Name: "app2-a",
			},
			"libA": {
				Dir:  turbopath.AnchoredUnixPath("libs/libA").ToSystemPath(),
				Name: "libA",
			},
			"libB": {
				Dir:                    turbopath.AnchoredUnixPath("libs/libB").ToSystemPath(),
				Name:                   "libB",
				UnresolvedExternalDeps: map[string]string{"external": "1"},
				TransitiveDeps: []lockfile.Package{
					{Key: "external-dep-a1", Version: "1", Found: true},
					{Key: "external-dep-b1", Version: "1", Found: true},
					{Key: "external1", Version: "1", Found: true},
				},
			},
			"libC": {
				Dir:  turbopath.AnchoredUnixPath("libs/libC").ToSystemPath(),
				Name: "libC",
			},
			"libD": {
				Dir:  turbopath.AnchoredUnixPath("libs/libD").ToSystemPath(),
				Name: "libD",
			},
		},
	}
	packageNames := []string{}
	for name := range workspaceInfos.PackageJSONs {
		packageNames = append(packageNames, name)
	}

	// global -> globalDep
	// app0-dep -> app0-dep :)

	makeLockfile := func(f func(*mockLockfile)) *mockLockfile {
		l := mockLockfile{
			globalChange: false,
			versions: map[string]string{
				"global":         "2",
				"app0-dep":       "2",
				"app0-util":      "2",
				"external":       "1",
				"external-dep-a": "1",
				"external-dep-b": "1",
			},
			allDeps: map[string]map[string]string{
				"global2": map[string]string{},
				"app0-dep2": map[string]string{
					"app0-util": "2",
				},
				"app0-util2": map[string]string{},
				"external1": map[string]string{
					"external-dep-a": "1",
					"external-dep-b": "1",
				},
				"external-dep-a1": map[string]string{},
				"external-dep-b1": map[string]string{},
			},
		}
		if f != nil {
			f(&l)
		}
		return &l
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
		lockfile            string
		currLockfile        *mockLockfile
		prevLockfile        *mockLockfile
		inferPkgPath        string
	}{
		{
			name:                "Just scope and dependencies",
			changed:             []string{},
			includeDependencies: true,
			scope:               []string{"app2"},
			expected:            []string{"app2", "libB", "libC", "libD"},
		},
		{
			name:                "Only turbo.json changed",
			changed:             []string{"turbo.json"},
			expected:            []string{"//", "app0", "app1", "app2", "app2-a", "libA", "libB", "libC", "libD"},
			since:               "dummy",
			includeDependencies: true,
		},
		{
			name:                "Only root package.json changed",
			changed:             []string{"package.json"},
			expected:            []string{"//", "app0", "app1", "app2", "app2-a", "libA", "libB", "libC", "libD"},
			since:               "dummy",
			includeDependencies: true,
		},
		{
			name:                "Only package-lock.json changed",
			changed:             []string{"package-lock.json"},
			expected:            []string{"//", "app0", "app1", "app2", "app2-a", "libA", "libB", "libC", "libD"},
			since:               "dummy",
			includeDependencies: true,
			lockfile:            "package-lock.json",
		},
		{
			name:                "Only yarn.lock changed",
			changed:             []string{"yarn.lock"},
			expected:            []string{"//", "app0", "app1", "app2", "app2-a", "libA", "libB", "libC", "libD"},
			since:               "dummy",
			includeDependencies: true,
			lockfile:            "yarn.lock",
		},
		{
			name:                "Only pnpm-lock.yaml changed",
			changed:             []string{"pnpm-lock.yaml"},
			expected:            []string{"//", "app0", "app1", "app2", "app2-a", "libA", "libB", "libC", "libD"},
			since:               "dummy",
			includeDependencies: true,
			lockfile:            "pnpm-lock.yaml",
		},
		{
			name:     "One package changed",
			changed:  []string{"libs/libB/src/index.ts"},
			expected: []string{"libB"},
			since:    "dummy",
		},
		{
			name:     "One package manifest changed",
			changed:  []string{"libs/libB/package.json"},
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
			expected:   []string{"//", "app0", "app1", "app2", "app2-a", "libA", "libB", "libC", "libD"},
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
			expected:          []string{"//", "app0", "app1", "app2", "app2-a", "libA", "libB", "libC", "libD"},
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
		{
			// make sure multiple apps with the same prefix are handled separately.
			// prevents this issue: https://github.com/vercel/turbo/issues/1528
			name:     "Two apps with an overlapping prefix changed",
			changed:  []string{"app/app2/src/index.js", "app/app2-a/src/index.js"},
			expected: []string{"app2", "app2-a"},
			since:    "dummy",
		},
		{
			name:         "Global lockfile change invalidates all packages",
			changed:      []string{"dummy.lock"},
			expected:     []string{"//", "app0", "app1", "app2", "app2-a", "libA", "libB", "libC", "libD"},
			lockfile:     "dummy.lock",
			currLockfile: makeLockfile(nil),
			prevLockfile: makeLockfile(func(ml *mockLockfile) {
				ml.globalChange = true
			}),
			since: "dummy",
		},
		{
			name:         "Dependency of workspace root change invalidates all packages",
			changed:      []string{"dummy.lock"},
			expected:     []string{"//", "app0", "app1", "app2", "app2-a", "libA", "libB", "libC", "libD"},
			lockfile:     "dummy.lock",
			currLockfile: makeLockfile(nil),
			prevLockfile: makeLockfile(func(ml *mockLockfile) {
				ml.versions["global"] = "3"
				ml.allDeps["global3"] = map[string]string{}
			}),
			since: "dummy",
		},
		{
			name:         "Version change invalidates package",
			changed:      []string{"dummy.lock"},
			expected:     []string{"//", "app0"},
			lockfile:     "dummy.lock",
			currLockfile: makeLockfile(nil),
			prevLockfile: makeLockfile(func(ml *mockLockfile) {
				ml.versions["app0-util"] = "3"
				ml.allDeps["app0-dep2"] = map[string]string{"app0-util": "3"}
				ml.allDeps["app0-util3"] = map[string]string{}
			}),
			since: "dummy",
		},
		{
			name:         "Transitive dep invalidates package",
			changed:      []string{"dummy.lock"},
			expected:     []string{"//", "libB"},
			lockfile:     "dummy.lock",
			currLockfile: makeLockfile(nil),
			prevLockfile: makeLockfile(func(ml *mockLockfile) {
				ml.versions["external-dep-a"] = "2"
				ml.allDeps["external1"] = map[string]string{"external-dep-a": "2", "external-dep-b": "1"}
				ml.allDeps["external-dep-a2"] = map[string]string{}
			}),
			since: "dummy",
		},
		{
			name:              "Transitive dep invalidates package and dependents",
			changed:           []string{"dummy.lock"},
			expected:          []string{"//", "app0", "app1", "app2", "libA", "libB"},
			lockfile:          "dummy.lock",
			includeDependents: true,
			currLockfile:      makeLockfile(nil),
			prevLockfile: makeLockfile(func(ml *mockLockfile) {
				ml.versions["external-dep-a"] = "2"
				ml.allDeps["external1"] = map[string]string{"external-dep-a": "2", "external-dep-b": "1"}
				ml.allDeps["external-dep-a2"] = map[string]string{}
			}),
			since: "dummy",
		},
		{
			name:         "Infer app2 from directory",
			inferPkgPath: "app/app2",
			expected:     []string{"app2"},
		},
		{
			name:         "Infer app2 from a subdirectory",
			inferPkgPath: "app/app2/src",
			expected:     []string{"app2"},
		},
		{
			name:         "Infer from a directory with no packages",
			inferPkgPath: "wrong",
			expected:     []string{},
		},
		{
			name:         "Infer from a parent directory",
			inferPkgPath: "app",
			expected:     []string{"app0", "app1", "app2", "app2-a"},
		},
		{
			name:         "library change, no scope, inferred libs",
			changed:      []string{"libs/libA/src/index.ts"},
			expected:     []string{"libA"},
			since:        "dummy",
			inferPkgPath: "libs",
		},
		{
			name:         "library change, no scope, inferred app",
			changed:      []string{"libs/libA/src/index.ts"},
			expected:     []string{},
			since:        "dummy",
			inferPkgPath: "app",
		},
	}
	for i, tc := range testCases {
		t.Run(fmt.Sprintf("test #%v %v", i, tc.name), func(t *testing.T) {
			// Convert test data to system separators.
			systemSeparatorChanged := make([]string, len(tc.changed))
			for index, path := range tc.changed {
				systemSeparatorChanged[index] = filepath.FromSlash(path)
			}
			scm := &mockSCM{
				changed:  systemSeparatorChanged,
				contents: make(map[string][]byte, len(systemSeparatorChanged)),
			}
			for _, path := range systemSeparatorChanged {
				scm.contents[path] = nil
			}
			readLockfile := func(_rootPackageJSON *fs.PackageJSON, content []byte) (lockfile.Lockfile, error) {
				return tc.prevLockfile, nil
			}
			pkgInferenceRoot, err := resolvePackageInferencePath(tc.inferPkgPath)
			if err != nil {
				t.Errorf("bad inference path (%v): %v", tc.inferPkgPath, err)
			}
			pkgs, isAllPackages, err := ResolvePackages(&Opts{
				LegacyFilter: LegacyFilter{
					Entrypoints:         tc.scope,
					Since:               tc.since,
					IncludeDependencies: tc.includeDependencies,
					SkipDependents:      !tc.includeDependents,
				},
				IgnorePatterns:       []string{tc.ignore},
				GlobalDepPatterns:    tc.globalDeps,
				PackageInferenceRoot: pkgInferenceRoot,
			}, root, scm, &context.Context{
				WorkspaceInfos: workspaceInfos,
				WorkspaceNames: packageNames,
				PackageManager: &packagemanager.PackageManager{Lockfile: tc.lockfile, UnmarshalLockfile: readLockfile},
				WorkspaceGraph: graph,
				RootNode:       "root",
				Lockfile:       tc.currLockfile,
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
