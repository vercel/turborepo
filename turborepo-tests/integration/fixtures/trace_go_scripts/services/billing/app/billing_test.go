package billing

import "testing"

func TestName(t *testing.T) {
	if Name() != "billing" {
		t.Fatal("billing")
	}
}
