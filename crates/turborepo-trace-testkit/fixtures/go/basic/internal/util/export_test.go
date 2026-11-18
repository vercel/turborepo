package util_test

import (
	"os"
	"testing"

	"example.com/basic/internal/util"
)

func TestGolden(t *testing.T) {
	os.ReadFile("testdata/golden.txt")
	_ = util.Name()
}
