package lockfile

import (
	"bytes"
	"testing"

	"gotest.tools/v3/assert"
)

func TestRoundtrip(t *testing.T) {
	content, err := getFixture(t, "yarn.lock")
	if err != nil {
		t.Error(err)
	}

	lockfile, err := DecodeYarnLockfile(content)
	if err != nil {
		t.Error(err)
	}

	var b bytes.Buffer
	if err := lockfile.Encode(&b); err != nil {
		t.Error(err)
	}

	assert.DeepEqual(t, string(content), b.String())
}

func TestKeySplitting(t *testing.T) {
	content, err := getFixture(t, "yarn.lock")
	if err != nil {
		t.Error(err)
	}

	lockfile, err := DecodeYarnLockfile(content)
	if err != nil {
		t.Error(err)
	}

	// @babel/types has multiple entries, these should all appear in the lockfile struct
	keys := []string{
		"@babel/types@^7.18.10",
		"@babel/types@^7.18.6",
		"@babel/types@^7.19.0",
	}

	for _, key := range keys {
		_, ok := lockfile.inner[key]
		assert.Assert(t, ok, "Unable to find entry for %s in parsed lockfile", key)
	}
}
