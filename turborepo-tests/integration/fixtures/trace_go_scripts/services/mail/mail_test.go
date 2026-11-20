package mail

import "testing"

func TestName(t *testing.T) {
	if Name() != "mail" {
		t.Fatal("mail")
	}
}
