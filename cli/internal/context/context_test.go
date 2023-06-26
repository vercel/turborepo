package context

import (
	"errors"
	"os"
	"path/filepath"
	"regexp"
	"sync"
	"testing"

	testifyAssert "github.com/stretchr/testify/assert"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/lockfile"
	"github.com/vercel/turbo/cli/internal/packagemanager"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/workspace"
	"gotest.tools/v3/assert"
)

func Test_isWorkspaceReference(t *testing.T) {
	rootpath, err := filepath.Abs(filepath.FromSlash("/some/repo"))
	if err != nil {
		t.Fatalf("failed to create absolute root path %v", err)
	}
	pkgDir, err := filepath.Abs(filepath.FromSlash("/some/repo/packages/libA"))
	if err != nil {
		t.Fatalf("failed to create absolute pkgDir %v", err)
	}
	tests := []struct {
		name              string
		packageVersion    string
		dependencyVersion string
		want              bool
	}{
		{
			name:              "handles exact match",
			packageVersion:    "1.2.3",
			dependencyVersion: "1.2.3",
			want:              true,
		},
		{
			name:              "handles semver range satisfied",
			packageVersion:    "1.2.3",
			dependencyVersion: "^1.0.0",
			want:              true,
		},
		{
			name:              "handles semver range not-satisfied",
			packageVersion:    "2.3.4",
			dependencyVersion: "^1.0.0",
			want:              false,
		},
		{
			name:              "handles workspace protocol with version",
			packageVersion:    "1.2.3",
			dependencyVersion: "workspace:1.2.3",
			want:              true,
		},
		{
			name:              "handles workspace protocol with relative path",
			packageVersion:    "1.2.3",
			dependencyVersion: "workspace:../other-package/",
			want:              true,
		},
		{
			name:              "handles npm protocol with satisfied semver range",
			packageVersion:    "1.2.3",
			dependencyVersion: "npm:^1.2.3",
			want:              true, // default in yarn is to use the workspace version unless `enableTransparentWorkspaces: true`. This isn't currently being checked.
		},
		{
			name:              "handles npm protocol with non-satisfied semver range",
			packageVersion:    "2.3.4",
			dependencyVersion: "npm:^1.2.3",
			want:              false,
		},
		{
			name:              "handles pre-release versions",
			packageVersion:    "1.2.3",
			dependencyVersion: "1.2.2-alpha-1234abcd.0",
			want:              false,
		},
		{
			name:              "handles non-semver package version",
			packageVersion:    "sometag",
			dependencyVersion: "1.2.3",
			want:              true, // for backwards compatability with the code before versions were verified
		},
		{
			name:              "handles non-semver package version",
			packageVersion:    "1.2.3",
			dependencyVersion: "sometag",
			want:              true, // for backwards compatability with the code before versions were verified
		},
		{
			name:              "handles file:... inside repo",
			packageVersion:    "1.2.3",
			dependencyVersion: "file:../libB",
			want:              true, // this is a sibling package
		},
		{
			name:              "handles file:... outside repo",
			packageVersion:    "1.2.3",
			dependencyVersion: "file:../../../otherproject",
			want:              false, // this is not within the repo root
		},
		{
			name:              "handles link:... inside repo",
			packageVersion:    "1.2.3",
			dependencyVersion: "link:../libB",
			want:              true, // this is a sibling package
		},
		{
			name:              "handles link:... outside repo",
			packageVersion:    "1.2.3",
			dependencyVersion: "link:../../../otherproject",
			want:              false, // this is not within the repo root
		},
		{
			name:              "handles development versions",
			packageVersion:    "0.0.0-development",
			dependencyVersion: "*",
			want:              true, // "*" should always match
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := isWorkspaceReference(tt.packageVersion, tt.dependencyVersion, pkgDir, rootpath)
			if got != tt.want {
				t.Errorf("isWorkspaceReference(%v, %v, %v, %v) got = %v, want %v", tt.packageVersion, tt.dependencyVersion, pkgDir, rootpath, got, tt.want)
			}
		})
	}
}

func TestBuildPackageGraph_DuplicateNames(t *testing.T) {
	path := getTestDir(t, "dupe-workspace-names")
	pkgJSON := &fs.PackageJSON{
		Name:           "dupe-workspace-names",
		PackageManager: "pnpm@7.15.0",
	}

	_, actualErr := BuildPackageGraph(path, pkgJSON, "pnpm")

	// Not asserting the full error message, because it includes a path with slashes and backslashes
	// getting the regex incantation to check that is not worth it.
	// We have to use regex because the actual error may be different depending on which workspace was
	// added first and which one was second, causing the error.
	testifyAssert.Regexp(t, regexp.MustCompile("^Failed to add workspace \"same-name\".+$"), actualErr)
}

func Test_populateExternalDeps_NoTransitiveDepsWithoutLockfile(t *testing.T) {
	path := getTestDir(t, "dupe-workspace-names")
	pkgJSON := &fs.PackageJSON{
		Name:           "dupe-workspace-names",
		PackageManager: "pnpm@7.15.0",
	}

	pm, err := packagemanager.GetPackageManager("pnpm")
	assert.NilError(t, err)
	pm.UnmarshalLockfile = func(rootPackageJSON *fs.PackageJSON, contents []byte) (lockfile.Lockfile, error) {
		return nil, errors.New("bad lockfile")
	}
	context := Context{
		WorkspaceInfos: workspace.Catalog{
			PackageJSONs: map[string]*fs.PackageJSON{
				"a": {},
			},
		},
		WorkspaceNames: []string{},
		PackageManager: pm,
		mutex:          sync.Mutex{},
	}
	var warnings Warnings
	err = context.populateExternalDeps(path, pkgJSON, &warnings)
	assert.NilError(t, err)

	assert.DeepEqual(t, pkgJSON.ExternalDepsHash, "")
	assert.DeepEqual(t, context.WorkspaceInfos.PackageJSONs["a"].ExternalDepsHash, "")
	assert.Assert(t, warnings.errorOrNil() != nil)
}

// This is duplicated from fs.turbo_json_test.go.
// I wasn't able to pull it into a helper file/package because
// it requires the `fs` package and it would cause cyclical dependencies
// when used in turbo_json_test.go and would require more changes to fix that.
func getTestDir(t *testing.T, testName string) turbopath.AbsoluteSystemPath {
	defaultCwd, err := os.Getwd()
	if err != nil {
		t.Errorf("failed to get cwd: %v", err)
	}
	cwd, err := fs.CheckedToAbsoluteSystemPath(defaultCwd)
	if err != nil {
		t.Fatalf("cwd is not an absolute directory %v: %v", defaultCwd, err)
	}

	return cwd.UntypedJoin("testdata", testName)
}
