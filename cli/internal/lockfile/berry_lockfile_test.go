package lockfile

import (
	"bytes"
	"testing"

	"github.com/vercel/turbo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
)

func getBerryLockfile(t *testing.T, filename string) *BerryLockfile {
	content, err := getFixture(t, filename)
	if err != nil {
		t.Error(err)
	}
	lockfile, err := DecodeBerryLockfile(content)
	if err != nil {
		t.Error(err)
	}
	return lockfile
}

func Test_DecodingBerryLockfile(t *testing.T) {
	lockfile := getBerryLockfile(t, "berry.lock")
	assert.Equal(t, lockfile.version, 6)
	assert.Equal(t, lockfile.cacheKey, "8c0")
}

func Test_ResolvePackage(t *testing.T) {
	lockfile := getBerryLockfile(t, "berry.lock")

	type Case struct {
		name    string
		semver  string
		key     string
		version string
		found   bool
	}

	cases := map[string]Case{
		"can resolve '||' semver syntax": {
			name:    "js-tokens",
			semver:  "^3.0.0 || ^4.0.0",
			key:     "js-tokens@npm:4.0.0",
			version: "4.0.0",
			found:   true,
		},
		"handles packages with multiple descriptors": {
			name:    "js-tokens",
			semver:  "^4.0.0",
			key:     "js-tokens@npm:4.0.0",
			version: "4.0.0",
			found:   true,
		},
		"doesn't find nonexistent descriptors": {
			name:   "@babel/code-frame",
			semver: "^7.12.11",
			found:  false,
		},
		"handles workspace packages": {
			name:    "eslint-config-custom",
			semver:  "*",
			key:     "eslint-config-custom@workspace:packages/eslint-config-custom",
			version: "0.0.0-use.local",
			found:   true,
		},
	}

	for testName, testCase := range cases {
		pkg, err := lockfile.ResolvePackage("some-pkg", testCase.name, testCase.semver)
		assert.NilError(t, err)
		if testCase.found {
			assert.Equal(t, pkg.Key, testCase.key, testName)
			assert.Equal(t, pkg.Version, testCase.version, testName)
		}
		assert.Equal(t, pkg.Found, testCase.found, testName)
	}
}

func Test_AllDependencies(t *testing.T) {
	lockfile := getBerryLockfile(t, "berry.lock")

	pkg, err := lockfile.ResolvePackage("some-pkg", "react-dom", "18.2.0")
	assert.NilError(t, err)
	assert.Assert(t, pkg.Found, "expected to find react-dom")
	deps, found := lockfile.AllDependencies(pkg.Key)
	assert.Assert(t, found, "expected lockfile key for react-dom to be present")
	assert.Equal(t, len(deps), 2, "expected to find all react-dom direct dependencies")
	for pkgName, version := range deps {
		pkg, err := lockfile.ResolvePackage("some-pkg", pkgName, version)
		assert.NilError(t, err, "error finding %s@%s", pkgName, version)
		assert.Assert(t, pkg.Found, "expected to find lockfile entry for %s@%s", pkgName, version)
	}
}

func Test_BerryPatchList(t *testing.T) {
	lockfile := getBerryLockfile(t, "berry.lock")

	var locator _Locator
	if err := locator.parseLocator("resolve@npm:2.0.0-next.4"); err != nil {
		t.Error(err)
	}

	patchLocator, ok := lockfile.patches[locator]
	assert.Assert(t, ok, "Expected to find patch locator")
	patch, ok := lockfile.packages[patchLocator]
	assert.Assert(t, ok, "Expected to find patch")
	assert.Equal(t, patch.Version, "2.0.0-next.4")
}

func Test_PackageExtensions(t *testing.T) {
	lockfile := getBerryLockfile(t, "berry.lock")

	expectedExtensions := map[_Descriptor]_void{}
	for _, extension := range []string{"@babel/types@npm:^7.8.3", "lodash@npm:4.17.21"} {
		var extensionDescriptor _Descriptor
		if err := extensionDescriptor.parseDescriptor(extension); err != nil {
			t.Error(err)
		}
		expectedExtensions[extensionDescriptor] = _void{}
	}

	assert.DeepEqual(t, lockfile.packageExtensions, expectedExtensions)
}

func Test_StringifyMetadata(t *testing.T) {
	metadata := BerryLockfileEntry{
		Version:  "6",
		CacheKey: "8c0",
	}
	lockfile := map[string]*BerryLockfileEntry{"__metadata": &metadata}

	var b bytes.Buffer
	err := _writeBerryLockfile(&b, lockfile)
	assert.Assert(t, err == nil)
	assert.Equal(t, b.String(), `
__metadata:
  version: 6
  cacheKey: 8c0
`)
}

func Test_BerryRoundtrip(t *testing.T) {
	content, err := getFixture(t, "berry.lock")
	if err != nil {
		t.Error(err)
	}
	lockfile, err := DecodeBerryLockfile(content)
	if err != nil {
		t.Error(err)
	}

	var b bytes.Buffer
	if err := lockfile.Encode(&b); err != nil {
		t.Error(err)
	}

	assert.Equal(t, b.String(), string(content))
}

func Test_PatchPathExtraction(t *testing.T) {
	type Case struct {
		locator   string
		patchPath string
		isPatch   bool
	}
	cases := []Case{
		{
			locator:   "lodash@patch:lodash@npm%3A4.17.21#./.yarn/patches/lodash-npm-4.17.21-6382451519.patch::version=4.17.21&hash=2c6e9e&locator=berry-patch%40workspace%3A.",
			patchPath: ".yarn/patches/lodash-npm-4.17.21-6382451519.patch",
			isPatch:   true,
		},
		{
			locator: "lodash@npm:4.17.21",
			isPatch: false,
		},
		{
			locator:   "resolve@patch:resolve@npm%3A2.0.0-next.4#~builtin<compat/resolve>::version=2.0.0-next.4&hash=07638b",
			patchPath: "~builtin<compat/resolve>",
			isPatch:   true,
		},
	}

	for _, testCase := range cases {
		var locator _Locator
		err := locator.parseLocator(testCase.locator)
		if err != nil {
			t.Error(err)
		}
		patchPath, isPatch := locator.patchPath()
		assert.Equal(t, isPatch, testCase.isPatch, locator)
		assert.Equal(t, patchPath, testCase.patchPath, locator)
	}
}

func Test_PatchPrimaryVersion(t *testing.T) {
	// todo write tests to make sure extraction actually works
	type TestCase struct {
		descriptor string
		version    string
		isPatch    bool
	}
	testCases := []TestCase{
		{
			descriptor: "lodash@patch:lodash@npm%3A4.17.21#./.yarn/patches/lodash-npm-4.17.21-6382451519.patch::locator=berry-patch%40workspace%3A.",
			version:    "npm:4.17.21",
			isPatch:    true,
		},
		{
			descriptor: "typescript@patch:typescript@^4.5.2#~builtin<compat/typescript>",
			version:    "npm:^4.5.2",
			isPatch:    true,
		},
		{
			descriptor: "react@npm:18.2.0",
			isPatch:    false,
		},
	}

	for _, testCase := range testCases {
		var d _Descriptor
		err := d.parseDescriptor(testCase.descriptor)
		assert.NilError(t, err, testCase.descriptor)
		actual, isPatch := d.primaryVersion()
		assert.Equal(t, isPatch, testCase.isPatch, testCase)
		if testCase.isPatch {
			assert.Equal(t, actual, testCase.version, testCase.descriptor)
		}
	}
}

func Test_BerryPruneDescriptors(t *testing.T) {
	lockfile := getBerryLockfile(t, "minimal-berry.lock")
	prunedLockfile, err := lockfile.Subgraph(
		[]turbopath.AnchoredSystemPath{
			turbopath.AnchoredUnixPath("packages/a").ToSystemPath(),
			turbopath.AnchoredUnixPath("packages/c").ToSystemPath(),
		},
		[]string{"lodash@npm:4.17.21"},
	)
	if err != nil {
		t.Error(err)
	}
	lockfileA := prunedLockfile.(*BerryLockfile)

	prunedLockfile, err = lockfile.Subgraph(
		[]turbopath.AnchoredSystemPath{
			turbopath.AnchoredUnixPath("packages/b").ToSystemPath(),
			turbopath.AnchoredUnixPath("packages/c").ToSystemPath(),
		},
		[]string{"lodash@npm:4.17.21"},
	)
	if err != nil {
		t.Error(err)
	}
	lockfileB := prunedLockfile.(*BerryLockfile)

	lodashIdent := _Ident{name: "lodash"}
	lodashA := _Descriptor{lodashIdent, "npm:^4.17.0"}
	lodashB := _Descriptor{lodashIdent, "npm:^3.0.0 || ^4.0.0"}

	lodashEntryA, hasLodashA := lockfileA.descriptors[lodashA]
	lodashEntryB, hasLodashB := lockfileB.descriptors[lodashB]

	assert.Assert(t, hasLodashA, "Expected lockfile a to have descriptor used by a")
	assert.Assert(t, hasLodashB, "Expected lockfile b to have descriptor used by b")
	assert.DeepEqual(t, lodashEntryA.reference, lodashEntryB.reference)

	_, lockfileAHasB := lockfileA.descriptors[lodashB]
	_, lockfileBHasA := lockfileB.descriptors[lodashA]
	assert.Assert(t, !lockfileAHasB, "Expected lockfile a not to have descriptor used by b")
	assert.Assert(t, !lockfileBHasA, "Expected lockfile b not to have descriptor used by a")
}
