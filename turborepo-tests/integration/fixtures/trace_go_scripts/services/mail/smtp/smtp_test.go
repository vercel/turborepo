package smtp

import "testing"

func TestName(t *testing.T) {
	if Name() != "smtp" {
		t.Fatal("smtp")
	}
}
