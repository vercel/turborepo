package client

import (
	"context"
	"crypto/x509"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/ioutil"
	"net/http"
	"net/url"
	"runtime"
	"strconv"
	"strings"
	"sync/atomic"
	"time"

	"github.com/hashicorp/go-retryablehttp"
)

type RemoteClient struct {
	config *ClientConfig
	// Number of failed requests before we stop trying to upload/download artifacts to the remote cache
	maxRemoteFailCount uint64
	// Must be used via atomic package
	currentFailCount uint64
	// An http client
	HttpClient *retryablehttp.Client
}

func newRemoteClient(config *ClientConfig) (*RemoteClient, error) {
	client := &RemoteClient{
		config:             config,
		maxRemoteFailCount: config.MaxRemoteFailCount,
		HttpClient: &retryablehttp.Client{
			HTTPClient: &http.Client{
				Timeout: time.Duration(20 * time.Second),
			},
			RetryWaitMin: 2 * time.Second,
			RetryWaitMax: 10 * time.Second,
			RetryMax:     2,
			Backoff:      retryablehttp.DefaultBackoff,
			Logger:       config.Logger,
		},
	}
	client.HttpClient.CheckRetry = client.checkRetry
	return client, nil
}

func (c *RemoteClient) retryCachePolicy(resp *http.Response, err error) (bool, error) {
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

func (c *RemoteClient) checkRetry(ctx context.Context, resp *http.Response, err error) (bool, error) {
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
func (c *RemoteClient) okToRequest() error {
	if atomic.LoadUint64(&c.currentFailCount) < c.maxRemoteFailCount {
		return nil
	}
	return ErrTooManyFailures
}

// IsLoggedIn returns true if we have a token and either a team id or team slug
func (c *RemoteClient) IsLoggedIn() bool {
	return c.config.Token != "" && (c.config.TeamId != "" || c.config.TeamSlug != "")
}

func (c *RemoteClient) SetToken(token string) {
	c.config.Token = token
}

func (c *RemoteClient) makeUrl(endpoint string) string {
	return fmt.Sprintf("%v%v", c.config.ApiUrl, endpoint)
}

func (c *RemoteClient) UserAgent() string {
	return fmt.Sprintf("turbo %v %v %v (%v)", c.config.TurboVersion, runtime.Version(), runtime.GOOS, runtime.GOARCH)
}

func (c *RemoteClient) PutArtifact(hash string, artifactBody interface{}, duration int, tag string) error {
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
	req, err := retryablehttp.NewRequest(http.MethodPut, c.makeUrl("/v8/artifacts/"+hash+encoded), artifactBody)
	req.Header.Set("Content-Type", "application/octet-stream")
	req.Header.Set("x-artifact-duration", fmt.Sprintf("%v", duration))
	req.Header.Set("Authorization", "Bearer "+c.config.Token)
	req.Header.Set("User-Agent", c.UserAgent())
	if tag != "" {
		req.Header.Set("x-artifact-tag", tag)
	}
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

func (c *RemoteClient) FetchArtifact(hash string, rawBody interface{}) (*ClientResponse, error) {
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
	req.Header.Set("Authorization", "Bearer "+c.config.Token)
	req.Header.Set("User-Agent", c.UserAgent())
	if err != nil {
		return nil, fmt.Errorf("invalid cache URL: %w", err)
	}

	resp, err := c.HttpClient.Do(req)
	statusCode := 200
	duration := -1
	tag := ""
	var body io.ReadCloser
	if err == nil {
		statusCode = resp.StatusCode
		// If present, extract the duration from the response.
		if resp.Header.Get("x-artifact-duration") != "" {
			intVar, atoiErr := strconv.Atoi(resp.Header.Get("x-artifact-duration"))
			if atoiErr != nil {
				err = fmt.Errorf("invalid x-artifact-duration header: %w", atoiErr)
			}
			duration = intVar
		}
		// If present, extract the tag from the response.
		if resp.Header.Get("x-artifact-tag") != "" {
			tag = resp.Header.Get("x-artifact-tag")
		}
		body = resp.Body
	}

	return &ClientResponse{
		StatusCode:       statusCode,
		ArtifactDuration: duration,
		Body:             body,
		Tag:              tag,
	}, err
}

func (c *RemoteClient) RecordAnalyticsEvents(events []map[string]interface{}) error {
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
	req.Header.Set("Authorization", "Bearer "+c.config.Token)
	req.Header.Set("User-Agent", c.UserAgent())
	resp, err := c.HttpClient.Do(req)
	if resp != nil && resp.StatusCode != http.StatusOK && resp.StatusCode != http.StatusCreated {
		b, _ := ioutil.ReadAll(resp.Body)
		return fmt.Errorf("%s", string(b))
	}
	return err
}

func (c *RemoteClient) addTeamParam(params *url.Values) {
	if c.config.TeamId != "" && strings.HasPrefix(c.config.TeamId, "team_") {
		params.Add("teamId", c.config.TeamId)
	}
	if c.config.TeamSlug != "" {
		params.Add("slug", c.config.TeamSlug)
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
func (c *RemoteClient) GetTeams() (*TeamsResponse, error) {
	teamsResponse := &TeamsResponse{}
	req, err := retryablehttp.NewRequest(http.MethodGet, c.makeUrl("/v2/teams?limit=100"), nil)
	if err != nil {
		return nil, err
	}

	req.Header.Set("User-Agent", c.UserAgent())
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+c.config.Token)
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
func (c *RemoteClient) GetUser() (*UserResponse, error) {
	userResponse := &UserResponse{}
	req, err := retryablehttp.NewRequest(http.MethodGet, c.makeUrl("/v2/user"), nil)
	if err != nil {
		return nil, err
	}

	req.Header.Set("User-Agent", c.UserAgent())
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+c.config.Token)
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

func (c *RemoteClient) VerifySSOToken(token string, tokenName string) (*VerifiedSSOUser, error) {
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
