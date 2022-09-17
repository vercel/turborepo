package cacheitem

import (
	"fmt"
	"testing"

	"github.com/vercel/turborepo/cli/internal/turbopath"
)

func TestCreate(t *testing.T) {
	archive, err := Create(turbopath.AbsoluteSystemPath("/Users/nathanhammond/out.tar.gz"))
	defer func() { _ = archive.Close() }()

	if err != nil {
		fmt.Printf("%v", err)
	}

	_ = archive.AddFile("/", "/Users/nathanhammond/.zprofile")
}
