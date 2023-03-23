// Package client implements some interfaces and convenience methods to interact with Vercel APIs and Remote Cache
package client

import (
	"context"
	"crypto/x509"
	"errors"
	"fmt"
	"io/ioutil"
	"net/http"
	"net/url"
	"runtime"
	"strings"
	"sync/atomic"
	"time"

	"github.com/hashicorp/go-hclog"
	"github.com/hashicorp/go-retryablehttp"
	"github.com/vercel/turbo/cli/internal/ci"
)

// APIClient is the main interface for making network requests to Vercel
type APIClient struct {
	// The api's base URL
	baseURL      string
	token        string
	turboVersion string

	// Must be used via atomic package
	currentFailCount uint64
	HTTPClient       *retryablehttp.Client
	teamID           string
	teamSlug         string
	// Whether or not to send preflight requests before uploads
	usePreflight bool
}

// ErrTooManyFailures is returned from remote cache API methods after `maxRemoteFailCount` errors have occurred
var ErrTooManyFailures = errors.New("skipping HTTP Request, too many failures have occurred")

// _maxRemoteFailCount is the number of failed requests before we stop trying to upload/download
// artifacts to the remote cache
const _maxRemoteFailCount = uint64(3)

// SetToken updates the APIClient's Token
func (c *APIClient) SetToken(token string) {
	c.token = token
}

// RemoteConfig holds the authentication and endpoint details for the API client
type RemoteConfig struct {
	Token    string
	TeamID   string
	TeamSlug string
	APIURL   string
}

// Opts holds values for configuring the behavior of the API client
type Opts struct {
	UsePreflight bool
	Timeout      uint64
}

// ClientTimeout Exported ClientTimeout used in run.go
const ClientTimeout uint64 = 20

// NewClient creates a new APIClient
func NewClient(remoteConfig RemoteConfig, logger hclog.Logger, turboVersion string, opts Opts) *APIClient {
	client := &APIClient{
		baseURL:      remoteConfig.APIURL,
		turboVersion: turboVersion,
		HTTPClient: &retryablehttp.Client{
			HTTPClient: &http.Client{
				Timeout: time.Duration(opts.Timeout) * time.Second,
			},
			RetryWaitMin: 2 * time.Second,
			RetryWaitMax: 10 * time.Second,
			RetryMax:     2,
			Backoff:      retryablehttp.DefaultBackoff,
			Logger:       logger,
		},
		token:        remoteConfig.Token,
		teamID:       remoteConfig.TeamID,
		teamSlug:     remoteConfig.TeamSlug,
		usePreflight: opts.UsePreflight,
	}
	client.HTTPClient.CheckRetry = client.checkRetry
	return client
}

// hasUser returns true if we have credentials for a user
func (c *APIClient) hasUser() bool {
	return c.token != ""
}

// IsLinked returns true if we have a user and linked team
func (c *APIClient) IsLinked() bool {
	return c.hasUser() && (c.teamID != "" || c.teamSlug != "")
}

// GetTeamID returns the currently configured team id
func (c *APIClient) GetTeamID() string {
	return c.teamID
}

func (c *APIClient) retryCachePolicy(resp *http.Response, err error) (bool, error) {
	if err != nil {
		if errors.As(err, &x509.UnknownAuthorityError{}) {
			// Don't retry if the error was due to TLS cert verification failure.
			atomic.AddUint64(&c.currentFailCount, 1)
			return false, err
		}
		atomic.AddUint64(&c.currentFailCount, 1)
		return true, nil
	}

	// 429 Too Many Requests is recoverable. Sometimes the server puts
	// a Retry-After response header to indicate when the server is
	// available to start processing request from client.
	if resp.StatusCode == http.StatusTooManyRequests {
		atomic.AddUint64(&c.currentFailCount, 1)
		return true, nil
	}

	// Check the response code. We retry on 500-range responses to allow
	// the server time to recover, as 500's are typically not permanent
	// errors and may relate to outages on the server side. This will catch
	// invalid response codes as well, like 0 and 999.
	if resp.StatusCode == 0 || (resp.StatusCode >= 500 && resp.StatusCode != 501) {
		atomic.AddUint64(&c.currentFailCount, 1)
		return true, fmt.Errorf("unexpected HTTP status %s", resp.Status)
	}

	// swallow the error and stop retrying
	return false, nil
}

func (c *APIClient) checkRetry(ctx context.Context, resp *http.Response, err error) (bool, error) {
	// do not retry on context.Canceled or context.DeadlineExceeded
	if ctx.Err() != nil {
		atomic.AddUint64(&c.currentFailCount, 1)
		return false, ctx.Err()
	}

	// we're squashing the error from the request and substituting any error that might come
	// from our retry policy.
	shouldRetry, err := c.retryCachePolicy(resp, err)
	if shouldRetry {
		// Our policy says it's ok to retry, but we need to check the failure count
		if retryErr := c.okToRequest(); retryErr != nil {
			return false, retryErr
		}
	}
	return shouldRetry, err
}

// okToRequest returns nil if it's ok to make a request, and returns the error to
// return to the caller if a request is not allowed
func (c *APIClient) okToRequest() error {
	if atomic.LoadUint64(&c.currentFailCount) < _maxRemoteFailCount {
		return nil
	}
	return ErrTooManyFailures
}

func (c *APIClient) makeURL(endpoint string) string {
	return fmt.Sprintf("%v%v", c.baseURL, endpoint)
}

func (c *APIClient) userAgent() string {
	return fmt.Sprintf("turbo %v %v %v (%v)", c.turboVersion, runtime.Version(), runtime.GOOS, runtime.GOARCH)
}

// doPreflight returns response with closed body, latest request url, and any errors to the caller
func (c *APIClient) doPreflight(requestURL string, requestMethod string, requestHeaders string) (*http.Response, string, error) {
	req, err := retryablehttp.NewRequest(http.MethodOptions, requestURL, nil)
	req.Header.Set("User-Agent", c.userAgent())
	req.Header.Set("Access-Control-Request-Method", requestMethod)
	req.Header.Set("Access-Control-Request-Headers", requestHeaders)
	req.Header.Set("Authorization", "Bearer "+c.token)
	if err != nil {
		return nil, requestURL, fmt.Errorf("[WARNING] Invalid cache URL: %w", err)
	}

	// If resp is not nil, ignore any errors
	//  because most likely unimportant for preflight to handle.
	// Let follow-up request handle potential errors.
	resp, err := c.HTTPClient.Do(req)
	if resp == nil {
		return resp, requestURL, err
	}
	defer resp.Body.Close() //nolint:golint,errcheck // nothing to do
	// The client will continue following 307, 308 redirects until it hits
	//  max redirects, gets an error, or gets a normal response.
	// Get the url from the Location header or get the url used in the last
	//  request (could have changed after following redirects).
	// Note that net/http client does not continue redirecting the preflight
	//  request with the OPTIONS method for 301, 302, and 303 redirects.
	//  See golang/go Issue 18570.
	if locationURL, err := resp.Location(); err == nil {
		requestURL = locationURL.String()
	} else {
		requestURL = resp.Request.URL.String()
	}
	return resp, requestURL, nil
}

func (c *APIClient) addTeamParam(params *url.Values) {
	if c.teamID != "" && strings.HasPrefix(c.teamID, "team_") {
		params.Add("teamId", c.teamID)
	}
	if c.teamSlug != "" {
		params.Add("slug", c.teamSlug)
	}
}

// JSONPatch sends a byte array (json.marshalled payload) to a given endpoint with PATCH
func (c *APIClient) JSONPatch(endpoint string, body []byte) ([]byte, error) {
	resp, err := c.request(endpoint, http.MethodPatch, body)
	if err != nil {
		return nil, err
	}

	rawResponse, err := ioutil.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("failed to read response %v", err)
	}
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("%s", string(rawResponse))
	}

	return rawResponse, nil
}

// JSONPost sends a byte array (json.marshalled payload) to a given endpoint with POST
func (c *APIClient) JSONPost(endpoint string, body []byte) ([]byte, error) {
	resp, err := c.request(endpoint, http.MethodPost, body)
	if err != nil {
		return nil, err
	}

	rawResponse, err := ioutil.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("failed to read response %v", err)
	}

	// For non 200/201 status codes, return the response body as an error
	if resp.StatusCode != http.StatusOK && resp.StatusCode != http.StatusCreated {
		return nil, fmt.Errorf("%s", string(rawResponse))
	}

	return rawResponse, nil
}

func (c *APIClient) request(endpoint string, method string, body []byte) (*http.Response, error) {
	if err := c.okToRequest(); err != nil {
		return nil, err
	}

	params := url.Values{}
	c.addTeamParam(&params)
	encoded := params.Encode()
	if encoded != "" {
		encoded = "?" + encoded
	}

	requestURL := c.makeURL(endpoint + encoded)

	allowAuth := true
	if c.usePreflight {
		resp, latestRequestURL, err := c.doPreflight(requestURL, method, "Authorization, User-Agent")
		if err != nil {
			return nil, fmt.Errorf("pre-flight request failed before trying to fetch files in HTTP cache: %w", err)
		}

		requestURL = latestRequestURL
		headers := resp.Header.Get("Access-Control-Allow-Headers")
		allowAuth = strings.Contains(strings.ToLower(headers), strings.ToLower("Authorization"))
	}

	req, err := retryablehttp.NewRequest(method, requestURL, body)
	if err != nil {
		return nil, err
	}

	// Set headers
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("User-Agent", c.userAgent())

	if allowAuth {
		req.Header.Set("Authorization", "Bearer "+c.token)
	}

	if ci.IsCi() {
		req.Header.Set("x-artifact-client-ci", ci.Constant())
	}

	resp, err := c.HTTPClient.Do(req)
	if err != nil {
		return nil, err
	}

	// If there isn't a response, something else probably went wrong
	if resp == nil {
		return nil, fmt.Errorf("response from %s is nil, something went wrong", requestURL)
	}

	return resp, nil
}
