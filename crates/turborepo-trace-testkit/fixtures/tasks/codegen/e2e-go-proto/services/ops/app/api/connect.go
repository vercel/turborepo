package api

import (
	opsv1 "example.test/ops/app/gen/acme/ops/v1"
)

// BlobID returns the id of a request built from generated protobuf types.
func BlobID(req *opsv1.GetBlobRequest) string {
	return req.GetId()
}
