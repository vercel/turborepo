package e2e

import "testing"

func TestServiceConfig(t *testing.T) {
	h := newHarness(t)
	if len(h.read(t, "config.json")) == 0 {
		t.Fatal("empty config")
	}
}
