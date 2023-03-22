package client

import (
	"encoding/json"
	"fmt"
	"io/ioutil"
	"net/http"
	"net/url"
	"strings"

	"github.com/hashicorp/go-retryablehttp"
	"github.com/vercel/turbo/cli/internal/ci"
)

// RecordAnalyticsEvents is a specific method for POSTing events to Vercel
func (c *APIClient) RecordAnalyticsEvents(events []map[string]interface{}) error {
	if err := c.okToRequest(); err != nil {
		return err
	}
	params := url.Values{}
	c.addTeamParam(&params)
	encoded := params.Encode()
	if encoded != "" {
		encoded = "?" + encoded
	}
	body, err := json.Marshal(events)
	if err != nil {
		return err
	}

	requestURL := c.makeURL("/v8/artifacts/events" + encoded)
	allowAuth := true
	if c.usePreflight {
		resp, latestRequestURL, err := c.doPreflight(requestURL, http.MethodPost, "Content-Type, Authorization, User-Agent")
		if err != nil {
			return fmt.Errorf("pre-flight request failed before trying to store in HTTP cache: %w", err)
		}
		requestURL = latestRequestURL
		headers := resp.Header.Get("Access-Control-Allow-Headers")
		allowAuth = strings.Contains(strings.ToLower(headers), strings.ToLower("Authorization"))
	}

	req, err := retryablehttp.NewRequest(http.MethodPost, requestURL, body)
	if err != nil {
		return err
	}
	req.Header.Set("Content-Type", "application/json")
	if allowAuth {
		req.Header.Set("Authorization", "Bearer "+c.token)
	}
	req.Header.Set("User-Agent", c.userAgent())
	if ci.IsCi() {
		req.Header.Set("x-artifact-client-ci", ci.Constant())
	}
	resp, err := c.HTTPClient.Do(req)
	if resp != nil && resp.StatusCode != http.StatusOK && resp.StatusCode != http.StatusCreated {
		b, _ := ioutil.ReadAll(resp.Body)
		return fmt.Errorf("%s", string(b))
	}
	return err
}
