package client

import (
	"bytes"
	"encoding/json"
	"io/ioutil"
	"net/http"
	"net/http/httptest"
	"reflect"
	"testing"

	"github.com/google/uuid"
	"github.com/hashicorp/go-hclog"
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

	apiClient, err := New(&ClientConfig{
		ApiUrl:             ts.URL,
		Logger:             hclog.Default(),
		TurboVersion:       "v1",
		TeamId:             "",
		TeamSlug:           "my-team-slug",
		MaxRemoteFailCount: 1,
	})
	if err != nil {
		t.Errorf("failed to create client %v", err)
	}
	apiClient.SetToken("my-token")

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
	apiClient, err := New(&ClientConfig{
		ApiUrl:             ts.URL + "/hash",
		Logger:             hclog.Default(),
		TurboVersion:       "v1",
		TeamId:             "",
		TeamSlug:           "my-team-slug",
		MaxRemoteFailCount: 1,
	})
	if err != nil {
		t.Errorf("failed to create client %v", err)
	}
	apiClient.SetToken("my-token")
	expectedArtifactBody := []byte("My string artifact")

	// Test Put Artifact
	apiClient.PutArtifact("hash", expectedArtifactBody, 500, "")
	testBody := <-ch
	if !bytes.Equal(expectedArtifactBody, testBody) {
		t.Errorf("Handler read '%v', wants '%v'", testBody, expectedArtifactBody)
	}

}
