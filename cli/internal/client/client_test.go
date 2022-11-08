package client

import (
	"bytes"
	"encoding/json"
	"errors"
	"io/ioutil"
	"net/http"
	"net/http/httptest"
	"reflect"
	"testing"

	"github.com/google/uuid"
	"github.com/hashicorp/go-hclog"
	"github.com/vercel/turbo/cli/internal/util"
)

func Test_sendToServer(t *testing.T) {
	ch := make(chan []byte, 1)
	ts := httptest.NewServer(
		http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
			defer req.Body.Close()
			b, err := ioutil.ReadAll(req.Body)
			if err != nil {
				t.Errorf("failed to read request %v", err)
			}
			ch <- b
			w.WriteHeader(200)
			w.Write([]byte{})
		}))
	defer ts.Close()

	remoteConfig := RemoteConfig{
		TeamSlug: "my-team-slug",
		APIURL:   ts.URL,
		Token:    "my-token",
	}
	apiClient := NewClient(remoteConfig, hclog.Default(), "v1", Opts{})

	myUUID, err := uuid.NewUUID()
	if err != nil {
		t.Errorf("failed to create uuid %v", err)
	}
	events := []map[string]interface{}{
		{
			"sessionId": myUUID.String(),
			"hash":      "foo",
			"source":    "LOCAL",
			"event":     "hit",
		},
		{
			"sessionId": myUUID.String(),
			"hash":      "bar",
			"source":    "REMOTE",
			"event":     "MISS",
		},
	}

	apiClient.RecordAnalyticsEvents(events)

	body := <-ch

	result := []map[string]interface{}{}
	err = json.Unmarshal(body, &result)
	if err != nil {
		t.Errorf("unmarshalling body %v", err)
	}
	if !reflect.DeepEqual(events, result) {
		t.Errorf("roundtrip got %v, want %v", result, events)
	}
}

func Test_PutArtifact(t *testing.T) {
	ch := make(chan []byte, 1)
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		defer req.Body.Close()
		b, err := ioutil.ReadAll(req.Body)
		if err != nil {
			t.Errorf("failed to read request %v", err)
		}
		ch <- b
		w.WriteHeader(200)
		w.Write([]byte{})
	}))
	defer ts.Close()

	// Set up test expected values
	remoteConfig := RemoteConfig{
		TeamSlug: "my-team-slug",
		APIURL:   ts.URL,
		Token:    "my-token",
	}
	apiClient := NewClient(remoteConfig, hclog.Default(), "v1", Opts{})
	expectedArtifactBody := []byte("My string artifact")

	// Test Put Artifact
	apiClient.PutArtifact("hash", expectedArtifactBody, 500, "")
	testBody := <-ch
	if !bytes.Equal(expectedArtifactBody, testBody) {
		t.Errorf("Handler read '%v', wants '%v'", testBody, expectedArtifactBody)
	}

}

func Test_PutWhenCachingDisabled(t *testing.T) {
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		defer func() { _ = req.Body.Close() }()
		w.WriteHeader(403)
		_, _ = w.Write([]byte("{\"code\": \"remote_caching_disabled\",\"message\":\"caching disabled\"}"))
	}))
	defer ts.Close()

	// Set up test expected values
	remoteConfig := RemoteConfig{
		TeamSlug: "my-team-slug",
		APIURL:   ts.URL,
		Token:    "my-token",
	}
	apiClient := NewClient(remoteConfig, hclog.Default(), "v1", Opts{})
	expectedArtifactBody := []byte("My string artifact")
	// Test Put Artifact
	err := apiClient.PutArtifact("hash", expectedArtifactBody, 500, "")
	cd := &util.CacheDisabledError{}
	if !errors.As(err, &cd) {
		t.Errorf("expected cache disabled error, got %v", err)
	}
	if cd.Status != util.CachingStatusDisabled {
		t.Errorf("caching status: expected %v, got %v", util.CachingStatusDisabled, cd.Status)
	}
}

func Test_FetchWhenCachingDisabled(t *testing.T) {
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		defer func() { _ = req.Body.Close() }()
		w.WriteHeader(403)
		_, _ = w.Write([]byte("{\"code\": \"remote_caching_disabled\",\"message\":\"caching disabled\"}"))
	}))
	defer ts.Close()

	// Set up test expected values
	remoteConfig := RemoteConfig{
		TeamSlug: "my-team-slug",
		APIURL:   ts.URL,
		Token:    "my-token",
	}
	apiClient := NewClient(remoteConfig, hclog.Default(), "v1", Opts{})
	// Test Put Artifact
	resp, err := apiClient.FetchArtifact("hash")
	cd := &util.CacheDisabledError{}
	if !errors.As(err, &cd) {
		t.Errorf("expected cache disabled error, got %v", err)
	}
	if cd.Status != util.CachingStatusDisabled {
		t.Errorf("caching status: expected %v, got %v", util.CachingStatusDisabled, cd.Status)
	}
	if resp != nil {
		t.Errorf("response got %v, want <nil>", resp)
	}
}
