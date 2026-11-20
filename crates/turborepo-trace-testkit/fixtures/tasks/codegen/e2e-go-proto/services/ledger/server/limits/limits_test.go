package limits

import "testing"

func TestMaxEntries(t *testing.T) {
	if MaxEntries <= 0 {
		t.Errorf("MaxEntries = %d", MaxEntries)
	}
}
