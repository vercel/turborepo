package service

import (
	"testing"

	"example.com/e2ego/mocks"
)

func TestGreet(t *testing.T) {
	m := &mocks.MockStore{Values: map[string]string{"name": "x"}}
	if Greet(m) != "hello x" {
		t.Fatal()
	}
}
