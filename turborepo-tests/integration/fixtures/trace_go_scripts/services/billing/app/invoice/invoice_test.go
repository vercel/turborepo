package invoice

import "testing"

func TestName(t *testing.T) {
	if Name() != "invoice" {
		t.Fatal("invoice")
	}
}
