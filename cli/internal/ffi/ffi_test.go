package ffi

import (
	"os"
	"testing"
)

func Test_ChangedFiles(t *testing.T) {
	output := ChangedFiles("", "bf0980ed1d816568e8258c206ddf45d9a7e93c4f", "c0d4854060b41269d8f3e9383a5af0539849c72f", true)
	t.Log(output)
}
