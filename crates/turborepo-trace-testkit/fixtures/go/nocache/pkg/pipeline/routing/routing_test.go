package routing

import "testing"

func TestRoute(t *testing.T) {
	if Route("log") != "sink" {
		t.Fatal("route")
	}
}
