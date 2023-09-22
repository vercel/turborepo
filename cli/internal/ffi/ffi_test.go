package ffi

import (
	"testing"

	"gotest.tools/v3/assert"
)

// This test is here to verify that we correctly handle zero length buffers
// with null data pointers.
func Test_EmptyBuffer(t *testing.T) {
	buffer := toBuffer(nil)
	bytes := toBytes(buffer)
	assert.DeepEqual(t, bytes, []byte{})
}
