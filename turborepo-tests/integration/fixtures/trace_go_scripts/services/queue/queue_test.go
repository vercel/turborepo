package queue

import "testing"

func TestName(t *testing.T) {
	if Name() != "queue" {
		t.Fatal("queue")
	}
}
