package client

import (
	"crypto/md5"
	"encoding/base64"
	"encoding/json"
	"io/ioutil"
	"net/http"
	"reflect"
	"strings"
	"testing"

	"github.com/google/uuid"
	"github.com/hashicorp/go-hclog"
)

func Test_sendToServer(t *testing.T) {
	handler := http.NewServeMux()
	ch := make(chan []byte, 1)
	handler.HandleFunc("/v8/artifacts/events", func(w http.ResponseWriter, req *http.Request) {
		defer req.Body.Close()
		b, err := ioutil.ReadAll(req.Body)
		if err != nil {
			t.Errorf("failed to read request %v", err)
		}
		ch <- b
		w.WriteHeader(200)
		w.Write([]byte{})
	})
	server := &http.Server{Addr: "localhost:8888", Handler: handler}
	go server.ListenAndServe()

	apiClient := NewClient("http://localhost:8888", hclog.Default(), "v1", "", "my-team-slug", 1)
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

	server.Close()
}

func Test_PutArtifact(t *testing.T) {
	handler := http.NewServeMux()
	ch := make(chan string, 2)
	hash := "hash"
	handler.HandleFunc("/v8/artifacts/"+hash, func(w http.ResponseWriter, req *http.Request) {
		defer req.Body.Close()
		b, err := ioutil.ReadAll(req.Body)
		if err != nil {
			t.Errorf("failed to read request %v", err)
		}
		ch <- string(b)
		trailerMd5 := req.Trailer.Get("Content-MD5")
		ch <- trailerMd5
		w.WriteHeader(200)
		w.Write([]byte{})
	})
	server := &http.Server{Addr: "localhost:8889", Handler: handler}
	go server.ListenAndServe()

	// Set up test expected values
	apiClient := NewClient("http://localhost:8889", hclog.Default(), "v1", "", "my-team-slug", 1)
	apiClient.SetToken("my-token")
	expectedArtifactBody := "My string artifact"
	artifactReader := strings.NewReader(expectedArtifactBody)
	md5Sum := md5.Sum([]byte(expectedArtifactBody))
	expectedMd5 := base64.StdEncoding.EncodeToString(md5Sum[:])

	// Test Put Artifact
	apiClient.PutArtifact(hash, 500, artifactReader)
	testBody := <-ch
	if expectedArtifactBody != testBody {
		t.Errorf("Handler read '%v', wants '%v'", testBody, expectedArtifactBody)
	}

	testMd5 := <-ch
	if expectedMd5 != testMd5 {
		t.Errorf("Handler read trailer '%v', wants '%v'", testMd5, expectedMd5)
	}

	server.Close()
}
