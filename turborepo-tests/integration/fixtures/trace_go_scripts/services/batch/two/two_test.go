package two

import "testing"

func TestName(t *testing.T) {
	if Name() != "two" {
		t.Fatal("two")
	}
}
