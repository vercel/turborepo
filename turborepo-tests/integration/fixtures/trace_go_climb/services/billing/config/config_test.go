package config

import "testing"

// Files the tests of this package expect next to them.
var expected = []string{"shared/defaults.json"}

func TestRootManifest(t *testing.T) {
	if _, err := Load("../../../package.json"); err != nil {
		t.Fatal(err)
	}
}

func TestSharedDefaults(t *testing.T) {
	if _, err := Load("../../../shared/defaults.json"); err != nil {
		t.Fatal(err)
	}
	for _, name := range expected {
		if _, err := Load("testdata/" + name); err != nil {
			t.Fatal(err)
		}
	}
}
