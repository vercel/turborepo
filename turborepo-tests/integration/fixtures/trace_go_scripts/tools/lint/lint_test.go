package lint

import "testing"

func TestName(t *testing.T) {
	if Name() != "lint" {
		t.Fatal("lint")
	}
}
