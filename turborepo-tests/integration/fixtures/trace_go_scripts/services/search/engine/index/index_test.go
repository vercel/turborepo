package index

import "testing"

func TestName(t *testing.T) {
	if Name() != "index" {
		t.Fatal("index")
	}
}
