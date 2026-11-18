package legacy

import "testing"

func TestValue(t *testing.T) {
	if Value != 1 {
		t.Fatal()
	}
}
