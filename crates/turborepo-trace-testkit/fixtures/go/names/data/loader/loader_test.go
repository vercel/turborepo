package loader

import "testing"

func TestGet(t *testing.T) {
	s := New("../fixtures")
	got, err := s.Get("golden.json")
	if err != nil || len(got) == 0 {
		t.Fatalf("get: %v", err)
	}
}
