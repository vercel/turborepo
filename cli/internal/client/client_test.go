package client

import (
	"encoding/json"
	"io/ioutil"
	"net/http"
	"testing"

	"github.com/google/uuid"
	"github.com/hashicorp/go-hclog"
	"github.com/vercel/turborepo/cli/internal/analytics"
)

type testEvent struct {
	Source string `json:"source"`
	Event  string `json:"event"`
	Hash   string `json:"hash"`
}

func Test_sendToServer(t *testing.T) {
	handler := http.NewServeMux()
	ch := make(chan []byte, 1)
	handler.HandleFunc("/artifacts/events", func(w http.ResponseWriter, req *http.Request) {
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

	payloads := []interface{}{}
	payloads = append(payloads, &testEvent{
		Hash:   "foo",
		Source: "LOCAL",
		Event:  "HIT",
	},
	)
	payloads = append(payloads, &testEvent{
		Hash:   "foo",
		Source: "REMOTE",
		Event:  "MISS",
	})

	myUUID, err := uuid.NewUUID()
	if err != nil {
		t.Errorf("failed to create uuid %v", err)
	}
	events := &analytics.Events{
		SessionID: myUUID,
		Payloads:  payloads,
	}
	apiClient.RecordAnalyticsEvents(events)

	body := <-ch

	// Rather than construct something that would properly map the inner interface{},
	// use the basic json representation to validate the payload
	result := map[string]interface{}{}
	err = json.Unmarshal(body, &result)
	if err != nil {
		t.Errorf("unmarshalling body %v", err)
	}
	if result["sessionId"] != myUUID.String() {
		t.Errorf("uuid got %v, want %v", result["uuid"], myUUID.String())
	}
	resultEvents, ok := (result["events"]).([]interface{})
	if !ok {
		t.Errorf("events got %v, want an array of json objects", result["events"])
	}
	if len(resultEvents) != 2 {
		t.Errorf("events length got %v, want 2", len(resultEvents))
	}
	lastEvent, ok := resultEvents[1].(map[string]interface{})
	if !ok {
		t.Errorf("last event got %v, want json object", resultEvents[1])
	}
	if lastEvent["source"] != "REMOTE" {
		t.Errorf("last event source got %v, want REMOTE", lastEvent["source"])
	}

	server.Close()
}
