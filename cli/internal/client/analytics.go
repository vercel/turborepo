package client

import (
	"encoding/json"
	"time"
)

// RecordAnalyticsEvents is a specific method for POSTing events to Vercel
func (c *APIClient) RecordAnalyticsEvents(events []map[string]interface{}, timeout time.Duration) error {
	body, err := json.Marshal(events)
	if err != nil {
		return err

	}

	// We don't care about the response here
	if _, err := c.JSONPost("/v8/artifacts/events", body, timeout); err != nil {
		return err
	}

	return nil
}
