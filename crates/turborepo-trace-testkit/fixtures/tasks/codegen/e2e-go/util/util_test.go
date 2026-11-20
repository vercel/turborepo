package util

import "testing"

func TestTwice(t *testing.T) {
	if Twice(2) != 4 {
		t.Fatal()
	}
}
