package worker

import "testing"

func TestName(t *testing.T) {
	if Name() != "worker" {
		t.Fatal("worker")
	}
}
