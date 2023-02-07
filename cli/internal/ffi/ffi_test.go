package ffi

import (
	"testing"
)

func Test_ChangedFiles(t *testing.T) {
	output := ChangedFiles("repo_root", "from_commit", "to_commit", true)
	t.Log(output)
}
