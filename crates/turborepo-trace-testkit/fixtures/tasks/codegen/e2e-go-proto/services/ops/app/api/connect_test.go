package api

import (
	"testing"

	opsv1 "example.test/ops/app/gen/acme/ops/v1"
)

func TestBlobID(t *testing.T) {
	if BlobID(&opsv1.GetBlobRequest{Id: "a"}) != "a" {
		t.Fatal("id")
	}
}
