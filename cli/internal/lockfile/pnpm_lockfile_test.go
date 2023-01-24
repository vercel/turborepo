package lockfile

import (
	"bytes"
	"os"
	"testing"

	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/yaml"
	"gotest.tools/v3/assert"
)

func getFixture(t *testing.T, name string) ([]byte, error) {
	defaultCwd, err := os.Getwd()
	if err != nil {
		t.Errorf("failed to get cwd: %v", err)
	}
	cwd, err := fs.CheckedToAbsoluteSystemPath(defaultCwd)
	if err != nil {
		t.Fatalf("cwd is not an absolute directory %v: %v", defaultCwd, err)
	}
	lockfilePath := cwd.UntypedJoin("testdata", name)
	if !lockfilePath.FileExists() {
		return nil, errors.Errorf("unable to find 'testdata/%s'", name)
	}
	return os.ReadFile(lockfilePath.ToString())
}

func Test_Roundtrip(t *testing.T) {
	lockfiles := []string{"pnpm6-workspace.yaml", "pnpm7-workspace.yaml", "pnpm8.yaml"}

	for _, lockfilePath := range lockfiles {
		lockfileContent, err := getFixture(t, lockfilePath)
		if err != nil {
			t.Errorf("failure getting fixture: %s", err)
		}
		lockfile, err := DecodePnpmLockfile(lockfileContent)
		if err != nil {
			t.Errorf("decoding failed %s", err)
		}
		var b bytes.Buffer
		if err := lockfile.Encode(&b); err != nil {
			t.Errorf("encoding failed %s", err)
		}
		newLockfile, err := DecodePnpmLockfile(b.Bytes())
		if err != nil {
			t.Errorf("decoding failed %s", err)
		}

		assert.DeepEqual(t, lockfile, newLockfile)
	}
}

func Test_SpecifierResolution(t *testing.T) {
	contents, err := getFixture(t, "pnpm7-workspace.yaml")
	if err != nil {
		t.Error(err)
	}
	untypedLockfile, err := DecodePnpmLockfile(contents)
	lockfile := untypedLockfile.(*PnpmLockfile)
	if err != nil {
		t.Errorf("failure decoding lockfile: %v", err)
	}

	type Case struct {
		workspacePath turbopath.AnchoredUnixPath
		pkg           string
		specifier     string
		version       string
		found         bool
		err           string
	}

	cases := []Case{
		{workspacePath: "apps/docs", pkg: "next", specifier: "12.2.5", version: "12.2.5_ir3quccc6i62x6qn6jjhyjjiey", found: true},
		{workspacePath: "apps/web", pkg: "next", specifier: "12.2.5", version: "12.2.5_ir3quccc6i62x6qn6jjhyjjiey", found: true},
		{workspacePath: "apps/web", pkg: "typescript", specifier: "^4.5.3", version: "4.8.3", found: true},
		{workspacePath: "apps/web", pkg: "lodash", specifier: "bad-tag", version: "", found: false},
		{workspacePath: "apps/web", pkg: "lodash", specifier: "^4.17.21", version: "4.17.21_ehchni3mpmovsvjxesffg2i5a4", found: true},
		{workspacePath: "", pkg: "turbo", specifier: "latest", version: "1.4.6", found: true},
		{workspacePath: "apps/bad_workspace", pkg: "turbo", specifier: "latest", version: "1.4.6", err: "no workspace 'apps/bad_workspace' found in lockfile"},
	}

	for _, testCase := range cases {
		actualVersion, actualFound, err := lockfile.resolveSpecifier(testCase.workspacePath, testCase.pkg, testCase.specifier)
		if testCase.err != "" {
			assert.Error(t, err, testCase.err)
		} else {
			assert.Equal(t, actualFound, testCase.found, "%s@%s", testCase.pkg, testCase.version)
			assert.Equal(t, actualVersion, testCase.version, "%s@%s", testCase.pkg, testCase.version)
		}
	}
}

func Test_SpecifierResolutionV6(t *testing.T) {
	contents, err := getFixture(t, "pnpm8.yaml")
	if err != nil {
		t.Error(err)
	}
	untypedLockfile, err := DecodePnpmLockfile(contents)
	lockfile := untypedLockfile.(*PnpmLockfileV6)
	if err != nil {
		t.Errorf("failure decoding lockfile: %v", err)
	}

	type Case struct {
		workspacePath turbopath.AnchoredUnixPath
		pkg           string
		specifier     string
		version       string
		found         bool
		err           string
	}

	cases := []Case{
		{workspacePath: "packages/a", pkg: "c", specifier: "workspace:*", version: "link:../c", found: true},
		{workspacePath: "packages/a", pkg: "is-odd", specifier: "^3.0.1", version: "3.0.1", found: true},
		{workspacePath: "packages/b", pkg: "is-odd", specifier: "^3.0.1", version: "3.0.1", err: "Unable to find resolved version for is-odd@^3.0.1 in packages/b"},
		{workspacePath: "apps/bad_workspace", pkg: "turbo", specifier: "latest", version: "1.4.6", err: "no workspace 'apps/bad_workspace' found in lockfile"},
	}

	for _, testCase := range cases {
		actualVersion, actualFound, err := lockfile.resolveSpecifier(testCase.workspacePath, testCase.pkg, testCase.specifier)
		if testCase.err != "" {
			assert.Error(t, err, testCase.err)
		} else {
			assert.Equal(t, actualFound, testCase.found, "%s@%s", testCase.pkg, testCase.version)
			assert.Equal(t, actualVersion, testCase.version, "%s@%s", testCase.pkg, testCase.version)
		}
	}
}

func Test_SubgraphInjectedPackages(t *testing.T) {
	contents, err := getFixture(t, "pnpm7-workspace.yaml")
	if err != nil {
		t.Error(err)
	}
	lockfile, err := DecodePnpmLockfile(contents)
	assert.NilError(t, err, "decode lockfile")

	packageWithInjectedPackage := turbopath.AnchoredUnixPath("apps/docs").ToSystemPath()

	prunedLockfile, err := lockfile.Subgraph([]turbopath.AnchoredSystemPath{packageWithInjectedPackage}, []string{})
	assert.NilError(t, err, "prune lockfile")

	pnpmLockfile, ok := prunedLockfile.(*PnpmLockfile)
	assert.Assert(t, ok, "got different lockfile impl")

	_, hasInjectedPackage := pnpmLockfile.Packages["file:packages/ui"]

	assert.Assert(t, hasInjectedPackage, "pruned lockfile is missing injected package")

}

func Test_DecodePnpmUnquotedURL(t *testing.T) {
	resolutionWithQuestionMark := `{integrity: sha512-deadbeef, tarball: path/to/tarball?foo=bar}`
	var resolution map[string]interface{}
	err := yaml.Unmarshal([]byte(resolutionWithQuestionMark), &resolution)
	assert.NilError(t, err, "valid package entry should be able to be decoded")
	assert.Equal(t, resolution["tarball"], "path/to/tarball?foo=bar")
}

func Test_PnpmLockfilePatches(t *testing.T) {
	contents, err := getFixture(t, "pnpm-patch.yaml")
	assert.NilError(t, err)

	lockfile, err := DecodePnpmLockfile(contents)
	assert.NilError(t, err)

	patches := lockfile.Patches()
	assert.Equal(t, len(patches), 2)
	assert.Equal(t, patches[0], turbopath.AnchoredUnixPath("patches/@babel__core@7.20.12.patch"))
	assert.Equal(t, patches[1], turbopath.AnchoredUnixPath("patches/is-odd@3.0.1.patch"))
}

func Test_PnpmPrunePatches(t *testing.T) {
	contents, err := getFixture(t, "pnpm-patch.yaml")
	assert.NilError(t, err)

	lockfile, err := DecodePnpmLockfile(contents)
	assert.NilError(t, err)

	prunedLockfile, err := lockfile.Subgraph(
		[]turbopath.AnchoredSystemPath{turbopath.AnchoredSystemPath("packages/dependency")},
		[]string{"/is-odd/3.0.1_nrrwwz7lemethtlvvm75r5bmhq", "/is-number/6.0.0", "/@babel-core/7.20.12_3hyn7hbvzkemudbydlwjmrb65y"},
	)
	assert.NilError(t, err)

	assert.Equal(t, len(prunedLockfile.Patches()), 2)
}

func Test_PnpmPrunePatchesV6(t *testing.T) {
	contents, err := getFixture(t, "pnpm-patch-v6.yaml")
	assert.NilError(t, err)

	lockfile, err := DecodePnpmLockfile(contents)
	assert.NilError(t, err)

	prunedLockfile, err := lockfile.Subgraph(
		[]turbopath.AnchoredSystemPath{turbopath.AnchoredSystemPath("packages/a")},
		[]string{"/lodash@4.17.21"},
	)
	assert.NilError(t, err)

	assert.Equal(t, len(prunedLockfile.Patches()), 1)

	prunedLockfile, err = lockfile.Subgraph(
		[]turbopath.AnchoredSystemPath{turbopath.AnchoredSystemPath("packages/b")},
		[]string{"/@babel/helper-string-parser@7.19.4"},
	)
	assert.NilError(t, err)

	assert.Equal(t, len(prunedLockfile.Patches()), 1)
}

func Test_PnpmAbsoluteDependency(t *testing.T) {
	type testCase struct {
		fixture string
		key     string
	}
	testcases := []testCase{
		{"pnpm-absolute.yaml", "/@scope/child/1.0.0"},
		{"pnpm-absolute-v6.yaml", "/@scope/child@1.0.0"},
	}
	for _, tc := range testcases {
		contents, err := getFixture(t, tc.fixture)
		assert.NilError(t, err, tc.fixture)

		lockfile, err := DecodePnpmLockfile(contents)
		assert.NilError(t, err, tc.fixture)

		pkg, err := lockfile.ResolvePackage(turbopath.AnchoredUnixPath("packages/a"), "child", tc.key)
		assert.NilError(t, err, "resolve")
		assert.Assert(t, pkg.Found, tc.fixture)
		assert.DeepEqual(t, pkg.Key, tc.key)
		assert.DeepEqual(t, pkg.Version, "1.0.0")
	}
}
