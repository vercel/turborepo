package lockfile

import (
	"bytes"
	"testing"

	"gotest.tools/v3/assert"
)

func getBerryLockfile(t *testing.T) *BerryLockfile {
	content, err := getFixture(t, "berry.lock")
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
	lockfile := getBerryLockfile(t)
	assert.Equal(t, lockfile.version, 6)
	assert.Equal(t, lockfile.cacheKey, 8)
}

func Test_ResolvePackage(t *testing.T) {
	lockfile := getBerryLockfile(t)

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
		key, version, found := lockfile.ResolvePackage(testCase.name, testCase.semver)
		if testCase.found {
			assert.Equal(t, key, testCase.key, testName)
			assert.Equal(t, version, testCase.version, testName)
		}
		assert.Equal(t, found, testCase.found, testName)
	}
}

func Test_AllDependencies(t *testing.T) {
	lockfile := getBerryLockfile(t)

	key, _, found := lockfile.ResolvePackage("react-dom", "18.2.0")
	assert.Assert(t, found, "expected to find react-dom")
	deps, found := lockfile.AllDependencies(key)
	assert.Assert(t, found, "expected lockfile key for react-dom to be present")
	assert.Equal(t, len(deps), 3, "expected to find all react-dom direct dependencies")
	for pkgName, version := range deps {
		_, _, found := lockfile.ResolvePackage(pkgName, version)
		assert.Assert(t, found, "expected to find lockfile entry for %s@%s", pkgName, version)
	}
}

func Test_StringifyMetadata(t *testing.T) {
	metadata := BerryLockfileEntry{
		Version:  "6",
		CacheKey: 8,
	}
	lockfile := map[string]*BerryLockfileEntry{"__metadata": &metadata}

	var b bytes.Buffer
	err := _writeBerryLockfile(&b, lockfile)
	assert.Assert(t, err == nil)
	assert.Equal(t, b.String(), `
__metadata:
  version: 6
  cacheKey: 8
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
