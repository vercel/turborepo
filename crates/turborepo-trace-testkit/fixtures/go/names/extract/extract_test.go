package extract

import (
	"os"
	"path/filepath"
	"testing"
)

func TestUnpack(t *testing.T) {
	root := t.TempDir()
	if err := Unpack(root, map[string]string{"conf/greeting.txt": "hello"}); err != nil {
		t.Fatal(err)
	}
	got, err := os.ReadFile(filepath.Join(root, "conf/greeting.txt"))
	if err != nil || string(got) != "hello" {
		t.Fatalf("greeting: %v", err)
	}
	if _, err := os.Stat(filepath.Join(root, "stray.txt")); !os.IsNotExist(err) {
		t.Fatalf("stray file: %v", err)
	}
}
