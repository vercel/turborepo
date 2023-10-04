//go:build windows
// +build windows

package fs

import "testing"

func TestDifferentVolumes(t *testing.T) {
	p1 := "C:\\some\\path"
	p2 := "D:\\other\\path"
	contains, err := DirContainsPath(p1, p2)
	if err != nil {
		t.Errorf("DirContainsPath got error %v, want <nil>", err)
	}
	if contains {
		t.Errorf("DirContainsPath got true, want false")
	}
}
