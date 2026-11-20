package emit

import (
	"context"
	"testing"
)

func TestEmit(t *testing.T) {
	if Emit(context.Background(), "log") == "" {
		t.Fatal("no route")
	}
}
