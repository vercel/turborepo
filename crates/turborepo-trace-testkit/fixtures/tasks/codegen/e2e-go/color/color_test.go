package color

import "testing"

func TestString(t *testing.T) {
	if Red.String() != "Red" {
		t.Fatal(Red.String())
	}
}
