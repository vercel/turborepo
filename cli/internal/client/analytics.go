package client

import (
	"context"
	"encoding/json"
)

// RecordAnalyticsEvents is a specific method for POSTing events to Vercel
func (c *APIClient) RecordAnalyticsEvents(ctx context.Context, events []map[string]interface{}) error {
	body, err := json.Marshal(events)
	if err != nil {
		return err

	}

	// We don't care about the response here
	if _, err := c.JSONPost(ctx, "/v8/artifacts/events", body); err != nil {
		return err
	}

	return nil
}
