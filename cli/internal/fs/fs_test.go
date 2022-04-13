package fs

import (
	"path/filepath"
	"testing"
)

func Test_DirContainsPath(t *testing.T) {
	parent, err := filepath.Abs(filepath.Join("some", "path"))
	if err != nil {
		t.Fatalf("failed to construct parent path %v", err)
	}
	testcases := []struct {
		target []string
		want   bool
	}{
		{
			[]string{"..", "elsewhere"},
			false,
		},
		{
			[]string{"sibling"},
			false,
		},
		{
			// The same path as parent
			[]string{"some", "path"},
			true,
		},
		{
			[]string{"some", "path", "..", "path", "inside", "parent"},
			true,
		},
		{
			[]string{"some", "path", "inside", "..", "inside", "parent"},
			true,
		},
		{
			[]string{"some", "path", "inside", "..", "..", "outside", "parent"},
			false,
		},
		{
			[]string{"some", "pathprefix"},
			false,
		},
	}
	for _, tc := range testcases {
		target, err := filepath.Abs(filepath.Join(tc.target...))
		if err != nil {
			t.Fatalf("failed to construct path for %v: %v", tc.target, err)
		}
		got, err := DirContainsPath(parent, target)
		if err != nil {
			t.Fatalf("failed to check ")
		}
		if got != tc.want {
			t.Errorf("DirContainsPath(%v, %v) got %v, want %v", parent, target, got, tc.want)
		}
	}
}
