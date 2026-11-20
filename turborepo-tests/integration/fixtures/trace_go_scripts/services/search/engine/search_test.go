package search

import "testing"

func TestName(t *testing.T) {
	if Name() != "search" {
		t.Fatal("search")
	}
}
