package extract

import (
	"os"
	"path/filepath"
	"testing"
)

func TestUnpack(t *testing.T) {
	root := t.TempDir()
	if err := Unpack(root, map[string]string{"etc/hello.txt": "hi"}); err != nil {
		t.Fatal(err)
	}
	got, err := os.ReadFile(filepath.Join(root, "etc/hello.txt"))
	if err != nil || string(got) != "hi" {
		t.Fatalf("read back: %v", err)
	}
	if _, err := os.Stat(filepath.Join(root, "doomed.txt")); !os.IsNotExist(err) {
		t.Fatalf("unexpected file: %v", err)
	}
}
