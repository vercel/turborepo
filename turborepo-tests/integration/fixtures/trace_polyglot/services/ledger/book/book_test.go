package book

import (
	"encoding/json"
	"os"
	"testing"
)

func TestTotal(t *testing.T) {
	if got := Total([]int64{1, 2, 3}); got != 6 {
		t.Fatalf("Total = %d", got)
	}
}

func TestRates(t *testing.T) {
	data, err := os.ReadFile("../../../shared/rates.json")
	if err != nil {
		t.Fatal(err)
	}
	var rates map[string]int64
	if err := json.Unmarshal(data, &rates); err != nil {
		t.Fatal(err)
	}
	if rates["EUR"] == 0 {
		t.Fatal("no EUR rate")
	}
}
