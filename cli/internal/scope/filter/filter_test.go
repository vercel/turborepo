package filter

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pyr-sh/dag"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/graph"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/util"
)

func setMatches(t *testing.T, name string, s util.Set, expected []string) {
	expectedSet := make(util.Set)
	for _, item := range expected {
		expectedSet.Add(item)
	}
	missing := s.Difference(expectedSet)
	if missing.Len() > 0 {
		t.Errorf("%v set has extra elements: %v", name, strings.Join(missing.UnsafeListOfStrings(), ", "))
	}
	extra := expectedSet.Difference(s)
	if extra.Len() > 0 {
		t.Errorf("%v set missing elements: %v", name, strings.Join(extra.UnsafeListOfStrings(), ", "))
	}
}

func Test_filter(t *testing.T) {
	root, err := os.Getwd()
	if err != nil {
		t.Fatalf("failed to get working directory: %v", err)
	}
	packageJSONs := make(graph.WorkspaceInfos)
	graph := &dag.AcyclicGraph{}
	graph.Add("project-0")
	packageJSONs["project-0"] = &fs.PackageJSON{
		Name: "project-0",
		Dir:  turbopath.AnchoredUnixPath("packages/project-0").ToSystemPath(),
	}
	graph.Add("project-1")
	packageJSONs["project-1"] = &fs.PackageJSON{
		Name: "project-1",
		Dir:  turbopath.AnchoredUnixPath("packages/project-1").ToSystemPath(),
	}
	graph.Add("project-2")
	packageJSONs["project-2"] = &fs.PackageJSON{
		Name: "project-2",
		Dir:  "project-2",
	}
	graph.Add("project-3")
	packageJSONs["project-3"] = &fs.PackageJSON{
		Name: "project-3",
		Dir:  "project-3",
	}
	graph.Add("project-4")
	packageJSONs["project-4"] = &fs.PackageJSON{
		Name: "project-4",
		Dir:  "project-4",
	}
	graph.Add("project-5")
	packageJSONs["project-5"] = &fs.PackageJSON{
		Name: "project-5",
		Dir:  "project-5",
	}
	// Note: inside project-5
	graph.Add("project-6")
	packageJSONs["project-6"] = &fs.PackageJSON{
		Name: "project-6",
		Dir:  turbopath.AnchoredUnixPath("project-5/packages/project-6").ToSystemPath(),
	}
	// Add dependencies
	graph.Connect(dag.BasicEdge("project-0", "project-1"))
	graph.Connect(dag.BasicEdge("project-0", "project-5"))
	graph.Connect(dag.BasicEdge("project-1", "project-2"))
	graph.Connect(dag.BasicEdge("project-1", "project-4"))

	r := &Resolver{
		Graph:          graph,
		WorkspaceInfos: packageJSONs,
		Cwd:            root,
	}

	testCases := []struct {
		Name      string
		Selectors []*TargetSelector
		Expected  []string
	}{
		{
			"select root package",
			[]*TargetSelector{
				{
					namePattern: util.RootPkgName,
				},
			},
			[]string{util.RootPkgName},
		},
		{
			"select only package dependencies (excluding the package itself)",
			[]*TargetSelector{
				{
					excludeSelf:         true,
					includeDependencies: true,
					namePattern:         "project-1",
				},
			},
			[]string{"project-2", "project-4"},
		},
		{
			"select package with dependencies",
			[]*TargetSelector{
				{
					excludeSelf:         false,
					includeDependencies: true,
					namePattern:         "project-1",
				},
			},
			[]string{"project-1", "project-2", "project-4"},
		},
		{
			"select package with dependencies and dependents, including dependent dependencies",
			[]*TargetSelector{
				{
					excludeSelf:         true,
					includeDependencies: true,
					includeDependents:   true,
					namePattern:         "project-1",
				},
			},
			[]string{"project-0", "project-1", "project-2", "project-4", "project-5"},
		},
		{
			"select package with dependents",
			[]*TargetSelector{
				{
					includeDependents: true,
					namePattern:       "project-2",
				},
			},
			[]string{"project-1", "project-2", "project-0"},
		},
		{
			"select dependents excluding package itself",
			[]*TargetSelector{
				{
					excludeSelf:       true,
					includeDependents: true,
					namePattern:       "project-2",
				},
			},
			[]string{"project-0", "project-1"},
		},
		{
			"filter using two selectors: one selects dependencies another selects dependents",
			[]*TargetSelector{
				{
					excludeSelf:       true,
					includeDependents: true,
					namePattern:       "project-2",
				},
				{
					excludeSelf:         true,
					includeDependencies: true,
					namePattern:         "project-1",
				},
			},
			[]string{"project-0", "project-1", "project-2", "project-4"},
		},
		{
			"select just a package by name",
			[]*TargetSelector{
				{
					namePattern: "project-2",
				},
			},
			[]string{"project-2"},
		},
		// Note: we don't support the option to switch path prefix mode
		// {
		// 	"select by parentDir",
		// 	[]*TargetSelector{
		// 		{
		// 			parentDir: "/packages",
		// 		},
		// 	},
		// 	[]string{"project-0", "project-1"},
		// },
		{
			"select by parentDir using glob",
			[]*TargetSelector{
				{
					parentDir: filepath.Join(root, "/packages/*"),
				},
			},
			[]string{"project-0", "project-1"},
		},
		{
			"select by parentDir using globstar",
			[]*TargetSelector{
				{
					parentDir: filepath.Join(root, "/project-5/**"),
				},
			},
			[]string{"project-5", "project-6"},
		},
		{
			"select by parentDir with no glob",
			[]*TargetSelector{
				{
					parentDir: filepath.Join(root, "/project-5"),
				},
			},
			[]string{"project-5"},
		},
		{
			"select all packages except one",
			[]*TargetSelector{
				{
					exclude:     true,
					namePattern: "project-1",
				},
			},
			[]string{"project-0", "project-2", "project-3", "project-4", "project-5", "project-6"},
		},
		{
			"select by parentDir and exclude one package by pattern",
			[]*TargetSelector{
				{
					parentDir: filepath.Join(root, "/packages/*"),
				},
				{
					exclude:     true,
					namePattern: "*-1",
				},
			},
			[]string{"project-0"},
		},
		{
			"select root package by directory",
			[]*TargetSelector{
				{
					parentDir: root,
				},
			},
			[]string{util.RootPkgName},
		},
	}

	for _, tc := range testCases {
		t.Run(tc.Name, func(t *testing.T) {
			pkgs, err := r.GetFilteredPackages(tc.Selectors)
			if err != nil {
				t.Fatalf("%v failed to filter packages: %v", tc.Name, err)
			}
			setMatches(t, tc.Name, pkgs.pkgs, tc.Expected)
		})
	}

	t.Run("report unmatched filters", func(t *testing.T) {
		pkgs, err := r.GetFilteredPackages([]*TargetSelector{
			{
				excludeSelf:         true,
				includeDependencies: true,
				namePattern:         "project-7",
			},
		})
		if err != nil {
			t.Fatalf("unmatched filter failed to filter packages: %v", err)
		}
		if pkgs.pkgs.Len() != 0 {
			t.Errorf("unmatched filter expected no packages, got %v", strings.Join(pkgs.pkgs.UnsafeListOfStrings(), ", "))
		}
		if len(pkgs.unusedFilters) != 1 {
			t.Errorf("unmatched filter expected to report one unused filter, got %v", len(pkgs.unusedFilters))
		}
	})
}

func Test_matchScopedPackage(t *testing.T) {
	root, err := os.Getwd()
	if err != nil {
		t.Fatalf("failed to get working directory: %v", err)
	}

	packageJSONs := make(graph.WorkspaceInfos)
	graph := &dag.AcyclicGraph{}
	graph.Add("@foo/bar")
	packageJSONs["@foo/bar"] = &fs.PackageJSON{
		Name: "@foo/bar",
		Dir:  turbopath.AnchoredUnixPath("packages/bar").ToSystemPath(),
	}
	r := &Resolver{
		Graph:          graph,
		WorkspaceInfos: packageJSONs,
		Cwd:            root,
	}
	pkgs, err := r.GetFilteredPackages([]*TargetSelector{
		{
			namePattern: "bar",
		},
	})
	if err != nil {
		t.Fatalf("failed to filter packages: %v", err)
	}
	setMatches(t, "match scoped package", pkgs.pkgs, []string{"@foo/bar"})
}

func Test_matchExactPackages(t *testing.T) {
	root, err := os.Getwd()
	if err != nil {
		t.Fatalf("failed to get working directory: %v", err)
	}

	packageJSONs := make(graph.WorkspaceInfos)
	graph := &dag.AcyclicGraph{}
	graph.Add("@foo/bar")
	packageJSONs["@foo/bar"] = &fs.PackageJSON{
		Name: "@foo/bar",
		Dir:  turbopath.AnchoredUnixPath("packages/@foo/bar").ToSystemPath(),
	}
	graph.Add("bar")
	packageJSONs["bar"] = &fs.PackageJSON{
		Name: "bar",
		Dir:  turbopath.AnchoredUnixPath("packages/bar").ToSystemPath(),
	}
	r := &Resolver{
		Graph:          graph,
		WorkspaceInfos: packageJSONs,
		Cwd:            root,
	}
	pkgs, err := r.GetFilteredPackages([]*TargetSelector{
		{
			namePattern: "bar",
		},
	})
	if err != nil {
		t.Fatalf("failed to filter packages: %v", err)
	}
	setMatches(t, "match exact package", pkgs.pkgs, []string{"bar"})
}

func Test_matchMultipleScopedPackages(t *testing.T) {
	root, err := os.Getwd()
	if err != nil {
		t.Fatalf("failed to get working directory: %v", err)
	}

	packageJSONs := make(graph.WorkspaceInfos)
	graph := &dag.AcyclicGraph{}
	graph.Add("@foo/bar")
	packageJSONs["@foo/bar"] = &fs.PackageJSON{
		Name: "@foo/bar",
		Dir:  turbopath.AnchoredUnixPath("packages/@foo/bar").ToSystemPath(),
	}
	graph.Add("@types/bar")
	packageJSONs["@types/bar"] = &fs.PackageJSON{
		Name: "@types/bar",
		Dir:  turbopath.AnchoredUnixPath("packages/@types/bar").ToSystemPath(),
	}
	r := &Resolver{
		Graph:          graph,
		WorkspaceInfos: packageJSONs,
		Cwd:            root,
	}
	pkgs, err := r.GetFilteredPackages([]*TargetSelector{
		{
			namePattern: "bar",
		},
	})
	if err != nil {
		t.Fatalf("failed to filter packages: %v", err)
	}
	setMatches(t, "match nothing with multiple scoped packages", pkgs.pkgs, []string{})
}

func Test_SCM(t *testing.T) {
	root, err := os.Getwd()
	if err != nil {
		t.Fatalf("failed to get working directory: %v", err)
	}
	head1Changed := make(util.Set)
	head1Changed.Add("package-1")
	head1Changed.Add("package-2")
	head1Changed.Add(util.RootPkgName)
	head2Changed := make(util.Set)
	head2Changed.Add("package-3")
	packageJSONs := make(graph.WorkspaceInfos)
	graph := &dag.AcyclicGraph{}
	graph.Add("package-1")
	packageJSONs["package-1"] = &fs.PackageJSON{
		Name: "package-1",
		Dir:  "package-1",
	}
	graph.Add("package-2")
	packageJSONs["package-2"] = &fs.PackageJSON{
		Name: "package-2",
		Dir:  "package-2",
	}
	graph.Add("package-3")
	packageJSONs["package-3"] = &fs.PackageJSON{
		Name: "package-3",
		Dir:  "package-3",
	}
	graph.Add("package-20")
	packageJSONs["package-20"] = &fs.PackageJSON{
		Name: "package-20",
		Dir:  "package-20",
	}

	graph.Connect(dag.BasicEdge("package-3", "package-20"))

	r := &Resolver{
		Graph:          graph,
		WorkspaceInfos: packageJSONs,
		Cwd:            root,
		PackagesChangedInRange: func(fromRef string, toRef string) (util.Set, error) {
			if fromRef == "HEAD~1" && toRef == "HEAD" {
				return head1Changed, nil
			} else if fromRef == "HEAD~2" && toRef == "HEAD" {
				union := head1Changed.Copy()
				for val := range head2Changed {
					union.Add(val)
				}
				return union, nil
			} else if fromRef == "HEAD~2" && toRef == "HEAD~1" {
				return head2Changed, nil
			}
			panic(fmt.Sprintf("unsupported commit range %v...%v", fromRef, toRef))
		},
	}

	testCases := []struct {
		Name      string
		Selectors []*TargetSelector
		Expected  []string
	}{
		{
			"all changed packages",
			[]*TargetSelector{
				{
					fromRef: "HEAD~1",
				},
			},
			[]string{"package-1", "package-2", util.RootPkgName},
		},
		{
			"all changed packages with parent dir exact match",
			[]*TargetSelector{
				{
					fromRef:   "HEAD~1",
					parentDir: root,
				},
			},
			[]string{util.RootPkgName},
		},
		{
			"changed packages in directory",
			[]*TargetSelector{
				{
					fromRef:   "HEAD~1",
					parentDir: filepath.Join(root, "package-2"),
				},
			},
			[]string{"package-2"},
		},
		{
			"changed packages matching pattern",
			[]*TargetSelector{
				{
					fromRef:     "HEAD~1",
					namePattern: "package-2*",
				},
			},
			[]string{"package-2"},
		},
		{
			"changed packages matching pattern",
			[]*TargetSelector{
				{
					fromRef:     "HEAD~1",
					namePattern: "package-2*",
				},
			},
			[]string{"package-2"},
		},
		// Note: missing test here that takes advantage of automatically exempting
		// test-only changes from pulling in dependents
		//
		// turbo-specific tests below here
		{
			"changed package was requested scope, and we're matching dependencies",
			[]*TargetSelector{
				{
					fromRef:           "HEAD~1",
					namePattern:       "package-1",
					matchDependencies: true,
				},
			},
			[]string{"package-1"},
		},
		{
			"older commit",
			[]*TargetSelector{
				{
					fromRef: "HEAD~2",
				},
			},
			[]string{"package-1", "package-2", "package-3", util.RootPkgName},
		},
		{
			"commit range",
			[]*TargetSelector{
				{
					fromRef:       "HEAD~2",
					toRefOverride: "HEAD~1",
				},
			},
			[]string{"package-3"},
		},
	}

	for _, tc := range testCases {
		t.Run(tc.Name, func(t *testing.T) {
			pkgs, err := r.GetFilteredPackages(tc.Selectors)
			if err != nil {
				t.Fatalf("%v failed to filter packages: %v", tc.Name, err)
			}
			setMatches(t, tc.Name, pkgs.pkgs, tc.Expected)
		})
	}
}
