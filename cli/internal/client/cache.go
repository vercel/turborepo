package client

import (
	"encoding/json"
	"fmt"
	"io"
	"io/ioutil"
	"net/http"
	"net/url"
	"strings"

	"github.com/hashicorp/go-retryablehttp"
	"github.com/vercel/turbo/cli/internal/ci"
	"github.com/vercel/turbo/cli/internal/util"
)

// PutArtifact uploads an artifact associated with a given hash string to the remote cache
func (c *APIClient) PutArtifact(hash string, artifactBody []byte, duration int, tag string) error {
	if err := c.okToRequest(); err != nil {
		return err
	}
	params := url.Values{}
	c.addTeamParam(&params)
	// only add a ? if it's actually needed (makes logging cleaner)
	encoded := params.Encode()
	if encoded != "" {
		encoded = "?" + encoded
	}

	requestURL := c.makeURL("/v8/artifacts/" + hash + encoded)
	allowAuth := true
	if c.usePreflight {
		resp, latestRequestURL, err := c.doPreflight(requestURL, http.MethodPut, "Content-Type, x-artifact-duration, Authorization, User-Agent, x-artifact-tag")
		if err != nil {
			return fmt.Errorf("pre-flight request failed before trying to store in HTTP cache: %w", err)
		}
		requestURL = latestRequestURL
		headers := resp.Header.Get("Access-Control-Allow-Headers")
		allowAuth = strings.Contains(strings.ToLower(headers), strings.ToLower("Authorization"))
	}

	req, err := retryablehttp.NewRequest(http.MethodPut, requestURL, artifactBody)
	req.Header.Set("Content-Type", "application/octet-stream")
	req.Header.Set("x-artifact-duration", fmt.Sprintf("%v", duration))
	if allowAuth {
		req.Header.Set("Authorization", "Bearer "+c.token)
	}
	req.Header.Set("User-Agent", c.userAgent())
	if ci.IsCi() {
		req.Header.Set("x-artifact-client-ci", ci.Constant())
	}
	if tag != "" {
		req.Header.Set("x-artifact-tag", tag)
	}
	if err != nil {
		return fmt.Errorf("[WARNING] Invalid cache URL: %w", err)
	}

	resp, err := c.HTTPClient.Do(req)
	if err != nil {
		return fmt.Errorf("[ERROR] Failed to store files in HTTP cache: %w", err)
	}
	defer func() { _ = resp.Body.Close() }()
	if resp.StatusCode == http.StatusForbidden {
		return c.handle403(resp.Body)
	}
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("[ERROR] Failed to store files in HTTP cache: %s against URL %s", resp.Status, requestURL)
	}
	return nil
}

// FetchArtifact attempts to retrieve the build artifact with the given hash from the remote cache
func (c *APIClient) FetchArtifact(hash string) (*http.Response, error) {
	return c.getArtifact(hash, http.MethodGet)
}

// ArtifactExists attempts to determine if the build artifact with the given hash exists in the Remote Caching server
func (c *APIClient) ArtifactExists(hash string) (*http.Response, error) {
	return c.getArtifact(hash, http.MethodHead)
}

// getArtifact attempts to retrieve the build artifact with the given hash from the remote cache
func (c *APIClient) getArtifact(hash string, httpMethod string) (*http.Response, error) {
	if httpMethod != http.MethodHead && httpMethod != http.MethodGet {
		return nil, fmt.Errorf("invalid httpMethod %v, expected GET or HEAD", httpMethod)
	}

	if err := c.okToRequest(); err != nil {
		return nil, err
	}
	params := url.Values{}
	c.addTeamParam(&params)
	// only add a ? if it's actually needed (makes logging cleaner)
	encoded := params.Encode()
	if encoded != "" {
		encoded = "?" + encoded
	}

	requestURL := c.makeURL("/v8/artifacts/" + hash + encoded)
	allowAuth := true
	if c.usePreflight {
		resp, latestRequestURL, err := c.doPreflight(requestURL, http.MethodGet, "Authorization, User-Agent")
		if err != nil {
			return nil, fmt.Errorf("pre-flight request failed before trying to fetch files in HTTP cache: %w", err)
		}
		requestURL = latestRequestURL
		headers := resp.Header.Get("Access-Control-Allow-Headers")
		allowAuth = strings.Contains(strings.ToLower(headers), strings.ToLower("Authorization"))
	}

	req, err := retryablehttp.NewRequest(httpMethod, requestURL, nil)
	if allowAuth {
		req.Header.Set("Authorization", "Bearer "+c.token)
	}
	req.Header.Set("User-Agent", c.userAgent())
	if err != nil {
		return nil, fmt.Errorf("invalid cache URL: %w", err)
	}

	resp, err := c.HTTPClient.Do(req)
	if err != nil {
		return nil, fmt.Errorf("failed to fetch artifact: %v", err)
	} else if resp.StatusCode == http.StatusForbidden {
		err = c.handle403(resp.Body)
		_ = resp.Body.Close()
		return nil, err
	}
	return resp, nil
}

func (c *APIClient) handle403(body io.Reader) error {
	raw, err := ioutil.ReadAll(body)
	if err != nil {
		return fmt.Errorf("failed to read response %v", err)
	}
	apiError := &apiError{}
	err = json.Unmarshal(raw, apiError)
	if err != nil {
		return fmt.Errorf("failed to read response (%v): %v", string(raw), err)
	}
	disabledErr, err := apiError.cacheDisabled()
	if err != nil {
		return err
	}
	return disabledErr
}

type apiError struct {
	Code    string `json:"code"`
	Message string `json:"message"`
}

func (ae *apiError) cacheDisabled() (*util.CacheDisabledError, error) {
	if strings.HasPrefix(ae.Code, "remote_caching_") {
		statusString := ae.Code[len("remote_caching_"):]
		status, err := util.CachingStatusFromString(statusString)
		if err != nil {
			return nil, err
		}
		return &util.CacheDisabledError{
			Status:  status,
			Message: ae.Message,
		}, nil
	}
	return nil, fmt.Errorf("unknown status %v: %v", ae.Code, ae.Message)
}
