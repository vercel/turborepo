package client

import (
	"context"
	"crypto/md5"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/ioutil"
	"net/http"
	"net/url"
	"runtime"
	"strings"
	"sync/atomic"
	"time"

	"github.com/hashicorp/go-hclog"
	"github.com/hashicorp/go-retryablehttp"
)

type ApiClient struct {
	// The api's base URL
	baseUrl      string
	Token        string
	turboVersion string
	// Number of failed requests before we stop trying to upload/download artifacts to the remote cache
	maxRemoteFailCount uint64
	// Must be used via atomic package
	currentFailCount uint64
	// An http client
	HttpClient *retryablehttp.Client
	teamID     string
	teamSlug   string
}

// ErrTooManyFailures is returned from remote cache API methods after `maxRemoteFailCount` errors have occurred
var ErrTooManyFailures = errors.New("skipping HTTP Request, too many failures have occurred")

func (api *ApiClient) SetToken(token string) {
	api.Token = token
}

// New creates a new ApiClient
func NewClient(baseUrl string, logger hclog.Logger, turboVersion string, teamID string, teamSlug string, maxRemoteFailCount uint64) *ApiClient {
	client := &ApiClient{
		baseUrl:            baseUrl,
		turboVersion:       turboVersion,
		maxRemoteFailCount: maxRemoteFailCount,
		HttpClient: &retryablehttp.Client{
			HTTPClient: &http.Client{
				Timeout: time.Duration(20 * time.Second),
			},
			RetryWaitMin: 2 * time.Second,
			RetryWaitMax: 10 * time.Second,
			RetryMax:     2,
			Backoff:      retryablehttp.DefaultBackoff,
			Logger:       logger,
		},
		teamID:   teamID,
		teamSlug: teamSlug,
	}
	client.HttpClient.CheckRetry = client.checkRetry
	return client
}

func (c *ApiClient) retryCachePolicy(resp *http.Response, err error) (bool, error) {
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

func (c *ApiClient) checkRetry(ctx context.Context, resp *http.Response, err error) (bool, error) {
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
func (c *ApiClient) okToRequest() error {
	if atomic.LoadUint64(&c.currentFailCount) < c.maxRemoteFailCount {
		return nil
	}
	return ErrTooManyFailures
}

func (c *ApiClient) makeUrl(endpoint string) string {
	return fmt.Sprintf("%v%v", c.baseUrl, endpoint)
}

func (c *ApiClient) UserAgent() string {
	return fmt.Sprintf("turbo %v %v %v (%v)", c.turboVersion, runtime.Version(), runtime.GOOS, runtime.GOARCH)
}

func (c *ApiClient) PutArtifact(hash string, duration int, artifactReader io.Reader) error {
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
	// Read the entire artifactReader into memory so we can easily compute the Content-MD5.
	// Note: retryablehttp.NewRequest reads the artifactReader into memory so there's no
	// additional overhead by doing the ioutil.ReadAll here instead.
	artifactBody, err := ioutil.ReadAll(artifactReader)
	if err != nil {
		return fmt.Errorf("failed to store files in HTTP cache: %w", err)
	}
	md5Sum := md5.Sum(artifactBody)
	contentMd5 := base64.StdEncoding.EncodeToString(md5Sum[:])

	req, err := retryablehttp.NewRequest(http.MethodPut, c.makeUrl("/v8/artifacts/"+hash+encoded), artifactBody)
	req.Header.Set("Content-Type", "application/octet-stream")
	req.Header.Set("x-artifact-duration", fmt.Sprintf("%v", duration))
	req.Header.Set("Authorization", "Bearer "+c.Token)
	req.Header.Set("User-Agent", c.UserAgent())
	req.Header.Set("Content-MD5", contentMd5)

	if err != nil {
		return fmt.Errorf("[WARNING] Invalid cache URL: %w", err)
	}
	if resp, err := c.HttpClient.Do(req); err != nil {
		return fmt.Errorf("failed to store files in HTTP cache: %w", err)
	} else {
		resp.Body.Close()
	}
	return nil
}

func (c *ApiClient) FetchArtifact(hash string, rawBody interface{}) (*http.Response, error) {
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
	req, err := retryablehttp.NewRequest(http.MethodGet, c.makeUrl("/v8/artifacts/"+hash+encoded), nil)
	req.Header.Set("Authorization", "Bearer "+c.Token)
	req.Header.Set("User-Agent", c.UserAgent())
	if err != nil {
		return nil, fmt.Errorf("invalid cache URL: %w", err)
	}

	return c.HttpClient.Do(req)
}

func (c *ApiClient) RecordAnalyticsEvents(events []map[string]interface{}) error {
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
	req, err := retryablehttp.NewRequest(http.MethodPost, c.makeUrl("/v8/artifacts/events"+encoded), body)
	if err != nil {
		return err
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+c.Token)
	req.Header.Set("User-Agent", c.UserAgent())
	resp, err := c.HttpClient.Do(req)
	if resp != nil && resp.StatusCode != http.StatusOK && resp.StatusCode != http.StatusCreated {
		b, _ := ioutil.ReadAll(resp.Body)
		return fmt.Errorf("%s", string(b))
	}
	return err
}

func (c *ApiClient) addTeamParam(params *url.Values) {
	if c.teamID != "" && strings.HasPrefix(c.teamID, "team_") {
		params.Add("teamId", c.teamID)
	}
	if c.teamSlug != "" {
		params.Add("slug", c.teamSlug)
	}
}

// Team is a Vercel Team object
type Team struct {
	ID        string `json:"id,omitempty"`
	Slug      string `json:"slug,omitempty"`
	Name      string `json:"name,omitempty"`
	CreatedAt int    `json:"createdAt,omitempty"`
	Created   string `json:"created,omitempty"`
}

// Pagination is a Vercel pagination object
type Pagination struct {
	Count int `json:"count,omitempty"`
	Next  int `json:"next,omitempty"`
	Prev  int `json:"prev,omitempty"`
}

// TeamsResponse is a Vercel object containing a list of teams and pagination info
type TeamsResponse struct {
	Teams      []Team     `json:"teams,omitempty"`
	Pagination Pagination `json:"pagination,omitempty"`
}

// GetTeams returns a list of Vercel teams
func (c *ApiClient) GetTeams() (*TeamsResponse, error) {
	teamsResponse := &TeamsResponse{}
	req, err := retryablehttp.NewRequest(http.MethodGet, c.makeUrl("/v2/teams?limit=100"), nil)
	if err != nil {
		return nil, err
	}

	req.Header.Set("User-Agent", c.UserAgent())
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+c.Token)
	resp, err := c.HttpClient.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode == http.StatusNotFound {
		return nil, fmt.Errorf("404 - Not found") // doesn't exist - not an error
	} else if resp.StatusCode != http.StatusOK {
		b, _ := ioutil.ReadAll(resp.Body)
		return nil, fmt.Errorf("%s", string(b))
	}
	body, readErr := ioutil.ReadAll(resp.Body)
	if readErr != nil {
		return nil, fmt.Errorf("could not read JSON response: %s", string(body))
	}
	marshalErr := json.Unmarshal(body, teamsResponse)
	if marshalErr != nil {
		return nil, fmt.Errorf("could not parse JSON response: %s", string(body))
	}
	return teamsResponse, nil
}

type User struct {
	ID        string `json:"id,omitempty"`
	Username  string `json:"username,omitempty"`
	Email     string `json:"email,omitempty"`
	Name      string `json:"name,omitempty"`
	CreatedAt int    `json:"createdAt,omitempty"`
}
type UserResponse struct {
	User User `json:"user,omitempty"`
}

// GetUser returns the current user
func (c *ApiClient) GetUser() (*UserResponse, error) {
	userResponse := &UserResponse{}
	req, err := retryablehttp.NewRequest(http.MethodGet, c.makeUrl("/v2/user"), nil)
	if err != nil {
		return nil, err
	}

	req.Header.Set("User-Agent", c.UserAgent())
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+c.Token)
	resp, err := c.HttpClient.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode == http.StatusNotFound {
		io.Copy(ioutil.Discard, resp.Body)
		return nil, fmt.Errorf("404 - Not found") // doesn't exist - not an error
	} else if resp.StatusCode != http.StatusOK {
		b, _ := ioutil.ReadAll(resp.Body)
		return nil, fmt.Errorf("%s", string(b))
	}
	body, readErr := ioutil.ReadAll(resp.Body)
	if readErr != nil {
		return nil, fmt.Errorf("could not read JSON response: %s", string(body))
	}
	marshalErr := json.Unmarshal(body, userResponse)
	if marshalErr != nil {
		return nil, fmt.Errorf("could not parse JSON response: %s", string(body))
	}
	return userResponse, nil
}

type verificationResponse struct {
	Token  string `json:"token"`
	Email  string `json:"email"`
	TeamID string `json:"teamId,omitempty"`
}

// VerifiedSSOUser contains data returned from the SSO token verification endpoint
type VerifiedSSOUser struct {
	Token  string
	TeamID string
}

func (c *ApiClient) VerifySSOToken(token string, tokenName string) (*VerifiedSSOUser, error) {
	query := make(url.Values)
	query.Add("token", token)
	query.Add("tokenName", tokenName)
	req, err := retryablehttp.NewRequest(http.MethodGet, c.makeUrl("/registration/verify")+"?"+query.Encode(), nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("User-Agent", c.UserAgent())
	resp, err := c.HttpClient.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode == http.StatusNotFound {
		io.Copy(ioutil.Discard, resp.Body)
		return nil, fmt.Errorf("404 - Not found") // doesn't exist - not an error
	} else if resp.StatusCode != http.StatusOK {
		b, _ := ioutil.ReadAll(resp.Body)
		return nil, fmt.Errorf("%s", string(b))
	}
	body, err := ioutil.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("could not read JSON response: %s", string(body))
	}
	verificationResponse := &verificationResponse{}
	err = json.Unmarshal(body, verificationResponse)
	if err != nil {
		return nil, fmt.Errorf("failed to unmarshall json response: %v", err)
	}
	vu := &VerifiedSSOUser{
		Token:  verificationResponse.Token,
		TeamID: verificationResponse.TeamID,
	}
	return vu, nil
}
