package one

import "testing"

func TestName(t *testing.T) {
	if Name() != "one" {
		t.Fatal("one")
	}
}
