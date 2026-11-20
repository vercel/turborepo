package tools

import "testing"

func TestName(t *testing.T) {
	if Name() != "tools" {
		t.Fatal("tools")
	}
}
