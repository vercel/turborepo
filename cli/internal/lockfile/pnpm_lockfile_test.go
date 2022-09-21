package lockfile

import (
	"bytes"
	"os"
	"testing"

	"github.com/pkg/errors"
	"github.com/vercel/turborepo/cli/internal/fs"
	"gotest.tools/v3/assert"
)

func getFixture(t *testing.T, name string) ([]byte, error) {
	defaultCwd, err := os.Getwd()
	if err != nil {
		t.Errorf("failed to get cwd: %v", err)
	}
	cwd, err := fs.CheckedToAbsolutePath(defaultCwd)
	if err != nil {
		t.Fatalf("cwd is not an absolute directory %v: %v", defaultCwd, err)
	}
	lockfilePath := cwd.Join("testdata", name)
	if !lockfilePath.FileExists() {
		return nil, errors.Errorf("unable to find 'testdata/%s'", name)
	}
	return os.ReadFile(lockfilePath.ToString())
}

func Test_Roundtrip(t *testing.T) {
	lockfiles := []string{"pnpm6-workspace.yaml", "pnpm7-workspace.yaml"}

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
	lockfile, err := DecodePnpmLockfile(contents)
	if err != nil {
		t.Errorf("failure decoding lockfile: %v", err)
	}

	type Case struct {
		pkg       string
		specifier string
		version   string
		found     bool
	}

	cases := []Case{
		{pkg: "next", specifier: "12.2.5", version: "12.2.5_ir3quccc6i62x6qn6jjhyjjiey", found: true},
		{pkg: "typescript", specifier: "^4.5.3", version: "4.8.3", found: true},
		{pkg: "lodash", specifier: "bad-tag", version: "", found: false},
		{pkg: "lodash", specifier: "^4.17.21", version: "4.17.21_ehchni3mpmovsvjxesffg2i5a4", found: true},
	}

	for _, testCase := range cases {
		actualVersion, actualFound := lockfile.resolveSpecifier(testCase.pkg, testCase.specifier)
		assert.Equal(t, actualFound, testCase.found, "%s@%s", testCase.pkg, testCase.version)
		assert.Equal(t, actualVersion, testCase.version, "%s@%s", testCase.pkg, testCase.version)
	}
}
