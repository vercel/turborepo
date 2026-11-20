package ops

import "testing"

func TestName(t *testing.T) {
	if Name() != "ops" {
		t.Fatal("ops")
	}
}
