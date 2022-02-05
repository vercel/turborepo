package client

import (
	"encoding/json"
	"fmt"
	"io"
	"io/ioutil"
	"net/http"
	"net/url"
	"runtime"
	"strings"
	"time"

	"github.com/hashicorp/go-hclog"
	"github.com/hashicorp/go-retryablehttp"
)

type ApiClient struct {
	// The api's base URL
	baseUrl      string
	Token        string
	turboVersion string
	// An http client
	HttpClient *retryablehttp.Client
}

func (api *ApiClient) SetToken(token string) {
	api.Token = token
}

// New creates a new ApiClient
func NewClient(baseUrl string, logger hclog.Logger, turboVersion string) *ApiClient {
	return &ApiClient{
		baseUrl:      baseUrl,
		turboVersion: turboVersion,
		HttpClient: &retryablehttp.Client{
			HTTPClient: &http.Client{
				Timeout: time.Duration(60 * time.Second),
			},
			RetryWaitMin: 10 * time.Second,
			RetryWaitMax: 20 * time.Second,
			RetryMax:     5,
			CheckRetry:   retryablehttp.DefaultRetryPolicy,
			Backoff:      retryablehttp.DefaultBackoff,
			Logger:       logger,
		},
	}
}

func (c *ApiClient) makeUrl(endpoint string) string {
	return fmt.Sprintf("%v%v", c.baseUrl, endpoint)
}

func (c *ApiClient) UserAgent() string {
	return fmt.Sprintf("turbo %v %v %v (%v)", c.turboVersion, runtime.Version(), runtime.GOOS, runtime.GOARCH)
}

func (c *ApiClient) PutArtifact(hash string, teamId string, slug string, duration int, rawBody interface{}) error {
	params := url.Values{}
	if teamId != "" && strings.HasPrefix(teamId, "team_") {
		params.Add("teamId", teamId)
	}
	if slug != "" {
		params.Add("slug", slug)
	}
	req, err := retryablehttp.NewRequest(http.MethodPut, c.makeUrl("/v8/artifacts/"+hash+"?"+params.Encode()), rawBody)
	req.Header.Set("Content-Type", "application/octet-stream")
	req.Header.Set("x-artifact-duration", fmt.Sprintf("%v", duration))
	req.Header.Set("Authorization", "Bearer "+c.Token)
	req.Header.Set("User-Agent", c.UserAgent())
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

func (c *ApiClient) FetchArtifact(hash string, teamId string, slug string, rawBody interface{}) (*http.Response, error) {
	params := url.Values{}
	if teamId != "" && strings.HasPrefix(teamId, "team_") {
		params.Add("teamId", teamId)
	}
	if slug != "" {
		params.Add("slug", slug)
	}
	req, err := retryablehttp.NewRequest(http.MethodGet, c.makeUrl("/v8/artifacts/"+hash+"?"+params.Encode()), nil)
	req.Header.Set("Authorization", "Bearer "+c.Token)
	req.Header.Set("User-Agent", c.UserAgent())
	if err != nil {
		return nil, fmt.Errorf("[WARNING] Invalid cache URL: %w", err)
	}
	return c.HttpClient.Do(req)
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
