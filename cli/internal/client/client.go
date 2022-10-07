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
	"strings"
	"sync/atomic"
	"time"

	"github.com/hashicorp/go-hclog"
	"github.com/hashicorp/go-retryablehttp"
	"github.com/spf13/pflag"
	"github.com/vercel/turborepo/cli/internal/util"
)

type ApiClient struct {
	// The api's base URL
	baseUrl      string
	token        string
	turboVersion string

	// Must be used via atomic package
	currentFailCount uint64
	// An http client
	HttpClient *retryablehttp.Client
	teamID     string
	teamSlug   string
	// Whether or not to send preflight requests before uploads
	usePreflight bool
}

// ErrTooManyFailures is returned from remote cache API methods after `maxRemoteFailCount` errors have occurred
var ErrTooManyFailures = errors.New("skipping HTTP Request, too many failures have occurred")

// _maxRemoteFailCount is the number of failed requests before we stop trying to upload/download
// artifacts to the remote cache
const _maxRemoteFailCount = uint64(3)

// SetToken updates the ApiClient's Token
func (c *ApiClient) SetToken(token string) {
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
}

// AddFlags adds flags specific to the api client to the given flagset
func AddFlags(opts *Opts, flags *pflag.FlagSet) {
	flags.BoolVar(&opts.UsePreflight, "preflight", false, "When enabled, turbo will precede HTTP requests with an OPTIONS request for authorization")
}

// New creates a new ApiClient
func NewClient(remoteConfig RemoteConfig, logger hclog.Logger, turboVersion string, opts Opts) *ApiClient {
	client := &ApiClient{
		baseUrl:      remoteConfig.APIURL,
		turboVersion: turboVersion,
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
		token:        remoteConfig.Token,
		teamID:       remoteConfig.TeamID,
		teamSlug:     remoteConfig.TeamSlug,
		usePreflight: opts.UsePreflight,
	}
	client.HttpClient.CheckRetry = client.checkRetry
	return client
}

// HasUser returns true if we have credentials for a user
func (c *ApiClient) HasUser() bool {
	return c.token != ""
}

// IsLinked returns true if we have a user and linked team
func (c *ApiClient) IsLinked() bool {
	return c.HasUser() && (c.teamID != "" || c.teamSlug != "")
}

// SetTeamID sets the team parameter used on all requests by this client
func (c *ApiClient) SetTeamID(teamID string) {
	c.teamID = teamID
}

// GetTeamID returns the currently configured team id
func (c *ApiClient) GetTeamID() string {
	return c.teamID
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
	if atomic.LoadUint64(&c.currentFailCount) < _maxRemoteFailCount {
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

// doPreflight returns response with closed body, latest request url, and any errors to the caller
func (c *ApiClient) doPreflight(requestURL string, requestMethod string, requestHeaders string) (*http.Response, string, error) {
	req, err := retryablehttp.NewRequest(http.MethodOptions, requestURL, nil)
	req.Header.Set("User-Agent", c.UserAgent())
	req.Header.Set("Access-Control-Request-Method", requestMethod)
	req.Header.Set("Access-Control-Request-Headers", requestHeaders)
	req.Header.Set("Authorization", "Bearer "+c.token)
	if err != nil {
		return nil, requestURL, fmt.Errorf("[WARNING] Invalid cache URL: %w", err)
	}

	// If resp is not nil, ignore any errors
	//  because most likely unimportant for preflight to handle.
	// Let follow-up request handle potential errors.
	resp, err := c.HttpClient.Do(req)
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

func (c *ApiClient) handle403(body io.Reader) error {
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

func (c *ApiClient) PutArtifact(hash string, artifactBody []byte, duration int, tag string) error {
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

	requestURL := c.makeUrl("/v8/artifacts/" + hash + encoded)
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
	req.Header.Set("User-Agent", c.UserAgent())
	if tag != "" {
		req.Header.Set("x-artifact-tag", tag)
	}
	if err != nil {
		return fmt.Errorf("[WARNING] Invalid cache URL: %w", err)
	}

	resp, err := c.HttpClient.Do(req)
	if err != nil {
		return fmt.Errorf("failed to store files in HTTP cache: %w", err)
	}
	defer func() { _ = resp.Body.Close() }()
	if resp.StatusCode == http.StatusForbidden {
		return c.handle403(resp.Body)
	}
	return nil
}

// FetchArtifact attempts to retrieve the build artifact with the given hash from the
// Remote Caching server
func (c *ApiClient) FetchArtifact(hash string) (*http.Response, error) {
	return c.getArtifact(hash, http.MethodGet)
}

// ArtifactExists attempts to determine if the build artifact with the given hash
// exists in the Remote Caching server
func (c *ApiClient) ArtifactExists(hash string) (*http.Response, error) {
	return c.getArtifact(hash, http.MethodHead)
}

// FetchArtifact attempts to retrieve the build artifact with the given hash from the
// Remote Caching server
func (c *ApiClient) getArtifact(hash string, httpMethod string) (*http.Response, error) {

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

	requestURL := c.makeUrl("/v8/artifacts/" + hash + encoded)
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
	req.Header.Set("User-Agent", c.UserAgent())
	if err != nil {
		return nil, fmt.Errorf("invalid cache URL: %w", err)
	}

	resp, err := c.HttpClient.Do(req)
	if err != nil {
		return nil, fmt.Errorf("failed to fetch artifact: %v", err)
	} else if resp.StatusCode == http.StatusForbidden {
		err = c.handle403(resp.Body)
		_ = resp.Body.Close()
		return nil, err
	}
	return resp, nil
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

	requestURL := c.makeUrl("/v8/artifacts/events" + encoded)
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

// Membership is the relationship between the logged-in user and a particular team
type Membership struct {
	Role string `json:"role"`
}

// Team is a Vercel Team object
type Team struct {
	ID         string     `json:"id,omitempty"`
	Slug       string     `json:"slug,omitempty"`
	Name       string     `json:"name,omitempty"`
	CreatedAt  int        `json:"createdAt,omitempty"`
	Created    string     `json:"created,omitempty"`
	Membership Membership `json:"membership"`
}

// IsOwner returns true if this Team data was fetched by an owner of the team
func (t *Team) IsOwner() bool {
	return t.Membership.Role == "OWNER"
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
	req.Header.Set("Authorization", "Bearer "+c.token)
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

// GetTeam gets a particular Vercel Team. It returns nil if it's not found
func (c *ApiClient) GetTeam(teamID string) (*Team, error) {
	queryParams := make(url.Values)
	queryParams.Add("teamId", teamID)
	req, err := retryablehttp.NewRequest(http.MethodGet, c.makeUrl("/v2/team?"+queryParams.Encode()), nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("User-Agent", c.UserAgent())
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+c.token)
	resp, err := c.HttpClient.Do(req)
	if err != nil {
		return nil, err
	}
	// We don't care if we fail to close the response body
	defer func() { _ = resp.Body.Close() }()
	if resp.StatusCode == http.StatusNotFound {
		return nil, nil // Doesn't exist, let calling code handle that case
	} else if resp.StatusCode != http.StatusOK {
		b, err := ioutil.ReadAll(resp.Body)
		var responseText string
		if err != nil {
			responseText = fmt.Sprintf("failed to read response: %v", err)
		} else {
			responseText = string(b)
		}
		return nil, fmt.Errorf("failed to get team (%v): %s", resp.StatusCode, responseText)
	}
	body, err := ioutil.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("failed to read team response: %v", err)
	}
	team := &Team{}
	err = json.Unmarshal(body, team)
	if err != nil {
		return nil, fmt.Errorf("failed to read JSON response: %v", string(body))
	}
	return team, nil
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
	req.Header.Set("Authorization", "Bearer "+c.token)
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

// statusResponse is the server response from /artifacts/status
type statusResponse struct {
	Status string `json:"status"`
}

// GetCachingStatus returns the server's perspective on whether or not remote caching
// requests will be allowed.
func (c *ApiClient) GetCachingStatus() (util.CachingStatus, error) {
	values := make(url.Values)
	c.addTeamParam(&values)
	req, err := retryablehttp.NewRequest(http.MethodGet, c.makeUrl("/v8/artifacts/status?"+values.Encode()), nil)
	if err != nil {
		return util.CachingStatusDisabled, err
	}
	req.Header.Set("User-Agent", c.UserAgent())
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+c.token)
	resp, err := c.HttpClient.Do(req)
	if err != nil {
		return util.CachingStatusDisabled, err
	}
	// Explicitly ignore the error from closing the response body. We don't need
	// to fail the method if we fail to close the response.
	defer func() { _ = resp.Body.Close() }()
	if resp.StatusCode != http.StatusOK {
		b, err := ioutil.ReadAll(resp.Body)
		var responseText string
		if err != nil {
			responseText = fmt.Sprintf("failed to read response: %v", err)
		} else {
			responseText = string(b)
		}
		return util.CachingStatusDisabled, fmt.Errorf("failed to get caching status (%v): %s", resp.StatusCode, responseText)
	}
	body, err := ioutil.ReadAll(resp.Body)
	if err != nil {
		return util.CachingStatusDisabled, fmt.Errorf("failed to read JSN response: %v", err)
	}
	statusResponse := statusResponse{}
	err = json.Unmarshal(body, &statusResponse)
	if err != nil {
		return util.CachingStatusDisabled, fmt.Errorf("failed to read JSON response: %v", string(body))
	}
	return util.CachingStatusFromString(statusResponse.Status)
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
