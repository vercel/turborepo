package emit

import (
	"context"

	"cloud.acme.test/go/storage/internal/apiv2"
	"example.test/pipeline/routing"
)

// Emit sends a record to the route of its kind.
func Emit(ctx context.Context, kind string) string {
	_ = apiv2.NewClient
	return routing.Route(kind)
}
