package parse

import (
	"testing"

	"github.com/acme-test/assert"
)

func TestFields(t *testing.T) {
	assert.Equal(t, 2, len(Fields("a b")))
}
