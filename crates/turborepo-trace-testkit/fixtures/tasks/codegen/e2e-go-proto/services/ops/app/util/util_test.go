package util

import "testing"

func TestChunk(t *testing.T) {
	if Chunk != 65536 {
		t.Fatal("chunk")
	}
}
